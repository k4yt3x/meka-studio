//! The Meka the app runs: its config, its token, and its process, started with the app, started
//! again when it ends, and stopped with the app.
//!
//! Meka runs `meka serve` on loopback with folders of the app's own, and nothing of any other Meka
//! on the computer: every `MEKA_` variable the app was started with is taken out of its
//! environment. It stops on SIGTERM, or on Windows on Ctrl+C, cancelling its turns and draining.

use std::{
    ffi::OsString,
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Mutex, mpsc},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use meka_studio::desktop::{MekaProcess, ProcessState};
use tokio::sync::watch;

use crate::files::{open_log, write_whole};

/// Every scope Meka grants, which the app's token holds.
const SCOPES: [&str; 12] = [
    "sessions:r",
    "sessions:w",
    "memory:r",
    "memory:w",
    "skills:r",
    "skills:w",
    "schedule:r",
    "schedule:w",
    "mcp:r",
    "mcp:w",
    "config:r",
    "config:w",
];

/// How long Meka has to stop: its drain of 30 seconds, then closing its MCP servers.
const STOP_WAIT: Duration = Duration::from_secs(45);

/// How often the process is looked at while it runs.
const POLL: Duration = Duration::from_millis(200);

/// The longest wait before Meka is started again after it ended.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// How long Meka must have run for an end to be taken as new, not as one of a series.
const STEADY: Duration = Duration::from_secs(60);

/// A new token for Meka, written to `file`, readable by its owner alone.
pub(crate) fn new_token(file: &Path) -> anyhow::Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| anyhow::anyhow!("no randomness: {error}"))?;
    let token = URL_SAFE_NO_PAD.encode(bytes);
    write_whole(file, token.as_bytes())?;
    Ok(token)
}

/// Makes sure Meka's `config.toml` in `config_dir` has a `[[serve.tokens]]` entry for
/// `token_file` with every scope, making the file or adding the entry if it is not there. The
/// rest of the file, which Meka writes itself, is left as written.
pub(crate) fn ensure_token_entry(config_dir: &Path, token_file: &Path) -> anyhow::Result<()> {
    let path = config_dir.join("config.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", path.display()));
        }
    };
    let mut document: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("{} is not valid TOML", path.display()))?;
    let token_file = token_file
        .to_str()
        .context("the token file's path is not UTF-8")?;
    let serve = document
        .entry("serve")
        .or_insert_with(toml_edit::table)
        .as_table_mut()
        .with_context(|| format!("[serve] in {} is not a table", path.display()))?;
    serve.set_implicit(true);
    let tokens = serve
        .entry("tokens")
        .or_insert_with(|| toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .with_context(|| {
            format!(
                "[serve].tokens in {} is not written as [[serve.tokens]]",
                path.display()
            )
        })?;
    let scopes = SCOPES.iter().copied().collect::<toml_edit::Array>();
    let has = |entry: &toml_edit::Table| {
        entry.get("token_file").and_then(toml_edit::Item::as_str) == Some(token_file)
    };
    let found = tokens.iter_mut().find(|entry| has(entry));
    match found {
        Some(entry) => {
            let current = entry
                .get("scopes")
                .and_then(toml_edit::Item::as_array)
                .map(|array| {
                    array
                        .iter()
                        .filter_map(|value| value.as_str())
                        .collect::<Vec<_>>()
                });
            if current.as_deref() == Some(&SCOPES[..]) {
                return Ok(());
            }
            entry.insert("scopes", toml_edit::value(scopes));
        }
        None => {
            let mut entry = toml_edit::Table::new();
            entry.insert("token_file", toml_edit::value(token_file));
            entry.insert("description", toml_edit::value("Meka Studio"));
            entry.insert("scopes", toml_edit::value(scopes));
            tokens.push(entry);
        }
    }
    write_whole(&path, document.to_string().as_bytes())
}

/// How to run Meka.
#[derive(Debug, Clone)]
pub(crate) struct Launch {
    /// The Meka binary.
    pub(crate) binary: PathBuf,
    /// Its config folder.
    pub(crate) config_dir: PathBuf,
    /// Its data folder.
    pub(crate) data_dir: PathBuf,
    /// The loopback address it serves on.
    pub(crate) address: SocketAddr,
    /// The file its output goes to.
    pub(crate) log: PathBuf,
    /// The file its process id is kept in while it runs, for the next start to find it by if the
    /// app ended without stopping it.
    pub(crate) pid_file: PathBuf,
    /// The folder it runs in, where new sessions open.
    pub(crate) home: PathBuf,
}

impl Launch {
    /// The command that runs Meka, with every `MEKA_` variable of `inherited` taken out and its
    /// folders given.
    fn command(&self, inherited: impl IntoIterator<Item = OsString>) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .arg("serve")
            .arg("--bind")
            .arg(self.address.to_string())
            .current_dir(&self.home)
            .stdin(Stdio::null());
        for name in inherited {
            // Windows reads a variable's name in any case.
            let meka = name
                .to_str()
                .and_then(|name| name.get(..5))
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("MEKA_"));
            if meka {
                command.env_remove(name);
            }
        }
        command
            .env("MEKA_CONFIG_DIR", &self.config_dir)
            .env("MEKA_DATA_DIR", &self.data_dir);
        command
    }

    /// Starts Meka, its output going to the log.
    fn spawn(&self) -> anyhow::Result<Child> {
        let log = open_log(&self.log)?;
        let mut command = self.command(std::env::vars_os().map(|(name, _)| name));
        command.stdout(log.try_clone()?).stderr(log);
        platform::prepare(&mut command);
        command
            .spawn()
            .with_context(|| format!("could not start {}", self.binary.display()))
    }
}

/// What the supervisor is told.
enum Order {
    Restart,
    Stop(mpsc::Sender<()>),
}

/// Meka's process, run by a thread of its own for as long as the app runs: on Linux, a process
/// started with a parent-death signal gets it when the thread that started it ends.
pub(crate) struct Meka {
    state: watch::Sender<MekaProcess>,
    orders: Mutex<mpsc::Sender<Order>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Meka {
    /// Starts Meka as `launch` says, and keeps it running.
    pub(crate) fn start(launch: Launch) -> anyhow::Result<Self> {
        let (state, _) = watch::channel(MekaProcess {
            state: ProcessState::Starting,
            pid: None,
            last_exit: None,
        });
        let (orders, inbox) = mpsc::channel();
        let thread = std::thread::Builder::new().name("meka".to_owned()).spawn({
            let state = state.clone();
            move || supervise(&launch, &state, &inbox)
        })?;
        Ok(Self {
            state,
            orders: Mutex::new(orders),
            thread: Mutex::new(Some(thread)),
        })
    }

    /// How Meka's process stands.
    pub(crate) fn state(&self) -> MekaProcess {
        self.state.borrow().clone()
    }

    /// How Meka's process stands, as it changes.
    pub(crate) fn watch(&self) -> watch::Receiver<MekaProcess> {
        self.state.subscribe()
    }

    /// Stops Meka and starts it again.
    pub(crate) fn restart(&self) {
        // Said at once, so whoever asked sees it under way.
        self.state
            .send_modify(|current| current.state = ProcessState::Restarting);
        self.order(Order::Restart);
    }

    /// Stops Meka, waiting until it has stopped.
    pub(crate) fn stop(&self) {
        let (done, stopped) = mpsc::channel();
        self.order(Order::Stop(done));
        if stopped.recv_timeout(STOP_WAIT * 2).is_err() {
            tracing::error!("Meka's supervisor did not report Meka stopped");
        }
        let thread = self
            .thread
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(thread) = thread
            && thread.join().is_err()
        {
            tracing::error!("Meka's supervisor panicked");
        }
    }

    fn order(&self, order: Order) {
        let sent = self
            .orders
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .send(order);
        if sent.is_err() {
            tracing::warn!("Meka's supervisor has stopped already");
        }
    }
}

/// How a run of Meka ended.
enum End {
    /// It ended by itself, or did not start.
    Exited,
    /// It was told to restart.
    Restart,
    /// It was told to stop.
    Stop(Option<mpsc::Sender<()>>),
}

/// Runs Meka until told to stop, starting it again when it ends.
fn supervise(launch: &Launch, state: &watch::Sender<MekaProcess>, inbox: &mpsc::Receiver<Order>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let set = |process: ProcessState, pid: Option<u32>| {
            state.send_modify(|current| {
                current.state = process;
                current.pid = pid;
            });
        };
        set(ProcessState::Starting, None);
        let started = Instant::now();
        let end = match launch.spawn() {
            Ok(child) => run(launch, child, state, inbox),
            Err(error) => {
                tracing::error!("{error:#}");
                state.send_modify(|current| current.last_exit = Some(format!("{error:#}")));
                End::Exited
            }
        };
        let done = match end {
            End::Stop(done) => done,
            End::Restart => {
                backoff = Duration::from_secs(1);
                continue;
            }
            End::Exited => {
                if started.elapsed() >= STEADY {
                    backoff = Duration::from_secs(1);
                }
                set(ProcessState::Restarting, None);
                match inbox.recv_timeout(backoff) {
                    Ok(Order::Restart) | Err(mpsc::RecvTimeoutError::Timeout) => {
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        continue;
                    }
                    Ok(Order::Stop(done)) => Some(done),
                    Err(mpsc::RecvTimeoutError::Disconnected) => None,
                }
            }
        };
        set(ProcessState::Stopped, None);
        if let Some(done) = done {
            // Whoever asked may have stopped waiting.
            let _ = done.send(());
        }
        return;
    }
}

/// Runs `child` until it ends or is told to, noting when it answers.
fn run(
    launch: &Launch,
    mut child: Child,
    state: &watch::Sender<MekaProcess>,
    inbox: &mpsc::Receiver<Order>,
) -> End {
    let pid = child.id();
    platform::adopt(&child);
    if let Err(error) = write_whole(&launch.pid_file, pid.to_string().as_bytes()) {
        tracing::warn!("could not note Meka's process id: {error:#}");
    }
    state.send_modify(|current| current.pid = Some(pid));
    tracing::info!("Meka started as process {pid}");
    let end = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                tracing::warn!("Meka ended: {status}");
                state.send_modify(|current| current.last_exit = Some(status.to_string()));
                return End::Exited;
            }
            Ok(None) => {}
            Err(error) => tracing::warn!("could not look at Meka's process: {error}"),
        }
        if state.borrow().state == ProcessState::Starting
            && TcpStream::connect_timeout(&launch.address, POLL).is_ok()
        {
            state.send_modify(|current| current.state = ProcessState::Running);
        }
        match inbox.recv_timeout(POLL) {
            Ok(Order::Restart) => break End::Restart,
            Ok(Order::Stop(done)) => break End::Stop(Some(done)),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            // The app is gone without saying so; Meka goes with it.
            Err(mpsc::RecvTimeoutError::Disconnected) => break End::Stop(None),
        }
    };
    match stop(&mut child) {
        Ok(status) => {
            tracing::info!("Meka stopped: {status}");
            state.send_modify(|current| current.last_exit = Some(status.to_string()));
            forget_pid(&launch.pid_file);
        }
        Err(error) => tracing::error!("could not stop Meka: {error:#}"),
    }
    end
}

/// Removes the file Meka's process id was kept in.
fn forget_pid(pid_file: &Path) {
    if let Err(error) = std::fs::remove_file(pid_file)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("could not remove {}: {error}", pid_file.display());
    }
}

/// Stops the Meka an earlier start of the app left running, if it ended without stopping it.
/// Linux ends it with the app by a parent-death signal, and Windows by a job; macOS has neither.
#[cfg(target_os = "macos")]
pub(crate) fn stop_leftover(launch: &Launch) {
    use std::os::unix::ffi::OsStrExt;
    let Some(pid) = std::fs::read_to_string(&launch.pid_file)
        .ok()
        .and_then(|text| text.trim().parse::<libc::pid_t>().ok())
    else {
        return;
    };
    let mut path = [0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: the buffer is as long as the length given.
    let length = unsafe {
        libc::proc_pidpath(
            pid,
            path.as_mut_ptr().cast(),
            u32::try_from(path.len()).unwrap_or(u32::MAX),
        )
    };
    let running = usize::try_from(length)
        .ok()
        .filter(|&length| length > 0)
        .and_then(|length| path.get(..length))
        .map(|bytes| std::path::Path::new(std::ffi::OsStr::from_bytes(bytes)).to_path_buf());
    let ours = |path: &Path| {
        std::fs::canonicalize(path).ok() == std::fs::canonicalize(&launch.binary).ok()
    };
    if running.as_deref().is_none_or(|running| !ours(running)) {
        forget_pid(&launch.pid_file);
        return;
    }
    tracing::warn!("stopping the Meka an earlier start left running, process {pid}");
    // SAFETY: plain signals to the process found running the app's Meka.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
        let asked = Instant::now();
        while libc::kill(pid, 0) == 0 && asked.elapsed() < STOP_WAIT {
            std::thread::sleep(POLL);
        }
        if libc::kill(pid, 0) == 0 {
            libc::kill(pid, libc::SIGKILL);
        }
    }
    forget_pid(&launch.pid_file);
}

/// Asks `child` to stop, and kills it if it has not within [`STOP_WAIT`].
fn stop(child: &mut Child) -> anyhow::Result<ExitStatus> {
    if let Err(error) = platform::ask_to_stop(child) {
        tracing::warn!("could not ask Meka to stop, so it is killed: {error:#}");
        child.kill()?;
        return Ok(child.wait()?);
    }
    let asked = Instant::now();
    while asked.elapsed() < STOP_WAIT {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        std::thread::sleep(POLL);
    }
    tracing::warn!("Meka did not stop within {STOP_WAIT:?}, so it is killed");
    child.kill()?;
    Ok(child.wait()?)
}

/// What a free loopback port is now, for Meka to serve on.
pub(crate) fn free_address() -> anyhow::Result<SocketAddr> {
    let probe = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(probe.local_addr()?)
}

/// Whether `binary` is there to run.
pub(crate) fn check_binary(binary: &Path) -> anyhow::Result<()> {
    if !binary.is_file() {
        bail!(
            "Meka is missing from the app: {} is not there. Install Meka Studio again.",
            binary.display()
        );
    }
    Ok(())
}

#[cfg(unix)]
mod platform {
    use std::process::{Child, Command};

    /// Starts Meka in a process group of its own, so a terminal's Ctrl+C reaches the app alone,
    /// and on Linux has it sent SIGTERM if the app ends without stopping it.
    pub(super) fn prepare(command: &mut Command) {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        #[cfg(target_os = "linux")]
        // SAFETY: `prctl` is async-signal-safe and touches nothing of the parent.
        unsafe {
            command.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    pub(super) const fn adopt(_child: &Child) {}

    /// Sends Meka SIGTERM, which it drains on.
    pub(super) fn ask_to_stop(child: &Child) -> anyhow::Result<()> {
        let pid = libc::pid_t::try_from(child.id())?;
        // SAFETY: a plain signal to the process this app started and has not yet reaped.
        if unsafe { libc::kill(pid, libc::SIGTERM) } == -1 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
}

#[cfg(windows)]
mod platform {
    use std::{
        os::windows::{io::AsRawHandle, process::CommandExt},
        process::{Child, Command},
        sync::OnceLock,
    };

    use windows_sys::Win32::{
        Foundation::{FALSE, HANDLE, TRUE},
        System::{
            Console::{
                AttachConsole, CTRL_C_EVENT, FreeConsole, GenerateConsoleCtrlEvent,
                SetConsoleCtrlHandler,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            },
            Threading::CREATE_NO_WINDOW,
        },
    };

    /// Starts Meka with a console of its own, which no window shows: Ctrl+C reaches it there.
    pub(super) fn prepare(command: &mut Command) {
        command.creation_flags(CREATE_NO_WINDOW);
    }

    /// A job that kills Meka when the app's process ends, however it ends.
    struct Job(HANDLE);

    // SAFETY: a job handle may be used from any thread.
    unsafe impl Send for Job {}
    // SAFETY: as above; the handle is only read.
    unsafe impl Sync for Job {}

    fn job() -> Option<&'static Job> {
        static JOB: OnceLock<Option<Job>> = OnceLock::new();
        JOB.get_or_init(|| {
            // SAFETY: a new unnamed job, and limits set on it alone.
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    tracing::warn!(
                        "could not make a job for Meka: {}",
                        std::io::Error::last_os_error()
                    );
                    return None;
                }
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let set = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&raw const limits).cast(),
                    u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                        .unwrap_or(u32::MAX),
                );
                if set == FALSE {
                    tracing::warn!(
                        "could not limit Meka's job: {}",
                        std::io::Error::last_os_error()
                    );
                    return None;
                }
                Some(Job(job))
            }
        })
        .as_ref()
    }

    /// Puts Meka in the job that ends it with the app.
    pub(super) fn adopt(child: &Child) {
        let Some(job) = job() else {
            return;
        };
        // SAFETY: the child's handle is open for as long as `child` lives.
        if unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle()) } == FALSE {
            tracing::warn!(
                "could not put Meka in its job: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    /// Sends Ctrl+C to Meka's console, which it drains on. The app has no console of its own to
    /// keep, and ignores Ctrl+C itself from then on.
    pub(super) fn ask_to_stop(child: &Child) -> anyhow::Result<()> {
        // SAFETY: console calls on this process's own console state.
        unsafe {
            FreeConsole();
            if AttachConsole(child.id()) == FALSE {
                return Err(std::io::Error::last_os_error().into());
            }
            SetConsoleCtrlHandler(None, TRUE);
            let sent = GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0);
            let error = std::io::Error::last_os_error();
            FreeConsole();
            if sent == FALSE {
                return Err(error.into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meka_is_given_folders_of_its_own_and_none_of_another_mekas() {
        let launch = Launch {
            binary: PathBuf::from("meka"),
            config_dir: PathBuf::from("/app/meka-config"),
            data_dir: PathBuf::from("/app/meka-data"),
            address: "127.0.0.1:41000".parse().unwrap(),
            log: PathBuf::from("/app/logs/meka.log"),
            pid_file: PathBuf::from("/app/meka.pid"),
            home: PathBuf::from("/home/me"),
        };
        let command = launch.command(
            [
                "MEKA_PERMISSION",
                "Meka_Profile",
                "MEKA_CONFIG_DIR",
                "PATH",
                "HOME",
            ]
            .map(OsString::from),
        );
        let envs: Vec<_> = command
            .get_envs()
            .map(|(name, value)| {
                (
                    name.to_string_lossy().into_owned(),
                    value.map(PathBuf::from),
                )
            })
            .collect();
        assert!(envs.contains(&("MEKA_PERMISSION".to_owned(), None)));
        assert!(envs.contains(&("Meka_Profile".to_owned(), None)));
        assert!(envs.contains(&(
            "MEKA_CONFIG_DIR".to_owned(),
            Some(PathBuf::from("/app/meka-config"))
        )));
        assert!(envs.contains(&(
            "MEKA_DATA_DIR".to_owned(),
            Some(PathBuf::from("/app/meka-data"))
        )));
        assert!(
            !envs
                .iter()
                .any(|(name, _)| name == "PATH" || name == "HOME")
        );
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["serve", "--bind", "127.0.0.1:41000"]);
    }

    #[test]
    fn the_token_entry_is_added_once_and_the_rest_of_the_config_kept() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        let token = dir.path().join("meka-token");
        std::fs::write(
            &config,
            "# My profiles\n[profiles.claude]\nmodel = \"m\"\n\n[serve]\nidle_timeout = \"1h\"\n",
        )
        .unwrap();
        ensure_token_entry(dir.path(), &token).unwrap();
        let first = std::fs::read_to_string(&config).unwrap();
        assert!(
            first.starts_with("# My profiles\n[profiles.claude]"),
            "{first}"
        );
        assert!(first.contains("idle_timeout = \"1h\""), "{first}");
        let parsed: toml_edit::DocumentMut = first.parse().unwrap();
        let entries = parsed["serve"]["tokens"].as_array_of_tables().unwrap();
        assert_eq!(entries.len(), 1);
        let entry = entries.get(0).unwrap();
        assert_eq!(entry["token_file"].as_str(), token.to_str());
        assert_eq!(entry["scopes"].as_array().unwrap().len(), SCOPES.len());
        ensure_token_entry(dir.path(), &token).unwrap();
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            first,
            "nothing to change"
        );
    }

    #[test]
    fn a_config_is_made_where_there_is_none_and_lost_scopes_are_given_back() {
        let dir = tempfile::tempdir().unwrap();
        let token = dir.path().join("meka-token");
        ensure_token_entry(dir.path(), &token).unwrap();
        let config = dir.path().join("config.toml");
        let made = std::fs::read_to_string(&config).unwrap();
        std::fs::write(&config, made.replace("\"config:w\"", "")).unwrap();
        ensure_token_entry(dir.path(), &token).unwrap();
        let parsed: toml_edit::DocumentMut =
            std::fs::read_to_string(&config).unwrap().parse().unwrap();
        let entry = parsed["serve"]["tokens"]
            .as_array_of_tables()
            .unwrap()
            .get(0)
            .unwrap()
            .clone();
        assert_eq!(entry["scopes"].as_array().unwrap().len(), SCOPES.len());
    }

    /// A stand-in Meka: a script that serves nothing, notes the signal it stops on, and ends.
    #[cfg(unix)]
    fn stand_in(dir: &Path) -> Launch {
        use std::os::unix::fs::PermissionsExt;
        let binary = dir.join("meka");
        std::fs::write(
            &binary,
            "#!/bin/sh\necho started >> \"$MEKA_DATA_DIR/runs\"\ntrap 'echo term >> \"$MEKA_DATA_DIR/runs\"; exit 0' TERM\nwhile :; do sleep 0.05; done\n",
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        Launch {
            binary,
            config_dir: dir.to_path_buf(),
            data_dir: dir.to_path_buf(),
            address: free_address().unwrap(),
            log: dir.join("meka.log"),
            pid_file: dir.join("meka.pid"),
            home: dir.to_path_buf(),
        }
    }

    /// Waits until `runs` holds `lines`.
    #[cfg(unix)]
    fn wait_for(runs: &Path, lines: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while std::fs::read_to_string(runs).unwrap_or_default() != lines {
            assert!(
                Instant::now() < deadline,
                "{:?}",
                std::fs::read_to_string(runs)
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    #[test]
    fn meka_is_asked_to_stop_on_restart_and_on_stop() {
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join("runs");
        let meka = Meka::start(stand_in(dir.path())).unwrap();
        wait_for(&runs, "started\n");
        assert!(meka.state().pid.is_some());
        meka.restart();
        wait_for(&runs, "started\nterm\nstarted\n");
        meka.stop();
        wait_for(&runs, "started\nterm\nstarted\nterm\n");
        assert_eq!(meka.state().state, ProcessState::Stopped);
    }

    #[cfg(unix)]
    #[test]
    fn meka_is_started_again_when_it_ends() {
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join("runs");
        let meka = Meka::start(stand_in(dir.path())).unwrap();
        wait_for(&runs, "started\n");
        let pid = libc::pid_t::try_from(meka.state().pid.unwrap()).unwrap();
        // SAFETY: a signal to the stand-in this test started.
        assert_eq!(unsafe { libc::kill(pid, libc::SIGKILL) }, 0);
        wait_for(&runs, "started\nstarted\n");
        assert!(
            meka.state()
                .last_exit
                .is_some_and(|exit| exit.contains("signal"))
        );
        meka.stop();
    }
}

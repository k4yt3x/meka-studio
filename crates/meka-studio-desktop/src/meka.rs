//! The Meka the app runs: its config, its token, and its process, started with the app, started
//! again when it ends, and stopped with the app.
//!
//! Meka runs `meka serve` on loopback with folders of the app's own, and nothing of any other Meka
//! on the computer: every `MEKA_` variable the app was started with is taken out of its
//! environment. Its standard input is a pipe the app holds open for as long as Meka should run, and
//! Meka cancels its turns, drains and ends when the pipe closes: the app closes it to stop Meka,
//! and the system closes it when the app ends however it ends.

use std::{
    ffi::OsString,
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
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

/// The description of the app's own `[[serve.tokens]]` entry, by which the app finds it wherever
/// its folders are.
const ENTRY: &str = "Meka Studio Desktop";

/// Makes sure Meka's `config.toml` in `config_dir` has the app's `[[serve.tokens]]` entry, the one
/// described as [`ENTRY`], naming `token_file` with every scope, making the file or adding the
/// entry if it is not there. An entry naming the token file where a moved portable copy kept it
/// is pointed at the one it has now, since Meka does not start while an entry's file is missing.
/// The rest of the file, which Meka writes itself, is left as written.
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
    let ours = |entry: &toml_edit::Table| {
        entry.get("description").and_then(toml_edit::Item::as_str) == Some(ENTRY)
    };
    let found = tokens.iter_mut().find(|entry| ours(entry));
    match found {
        Some(entry) => {
            let named = entry.get("token_file").and_then(toml_edit::Item::as_str);
            let current = entry
                .get("scopes")
                .and_then(toml_edit::Item::as_array)
                .map(|array| {
                    array
                        .iter()
                        .filter_map(|value| value.as_str())
                        .collect::<Vec<_>>()
                });
            if named == Some(token_file) && current.as_deref() == Some(&SCOPES[..]) {
                return Ok(());
            }
            entry.insert("token_file", toml_edit::value(token_file));
            entry.insert("scopes", toml_edit::value(scopes));
        }
        None => {
            let mut entry = toml_edit::Table::new();
            entry.insert("token_file", toml_edit::value(token_file));
            entry.insert("description", toml_edit::value(ENTRY));
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
    /// The folder it runs in, where new sessions open.
    pub(crate) home: PathBuf,
}

impl Launch {
    /// The command that runs Meka, with every `MEKA_` variable of `inherited` taken out, its
    /// folders given, and its standard input a pipe it ends on when the pipe closes.
    fn command(&self, inherited: impl IntoIterator<Item = OsString>) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .arg("serve")
            .arg("--bind")
            .arg(self.address.to_string())
            .arg("--exit-on-stdin-close")
            .current_dir(&self.home)
            .stdin(Stdio::piped());
        // In a process group of its own, so a terminal's Ctrl+C reaches the app alone.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        // Meka is a console program: without this, Windows would open a window for its console.
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(
            &mut command,
            windows_sys::Win32::System::Threading::CREATE_NO_WINDOW,
        );
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

/// Meka's process, run by a thread of its own for as long as the app runs.
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
    // Held until Meka is to stop: closed, it is what tells Meka to.
    let input = child.stdin.take();
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
    match stop(&mut child, input) {
        Ok(status) => {
            tracing::info!("Meka stopped: {status}");
            state.send_modify(|current| current.last_exit = Some(status.to_string()));
        }
        Err(error) => tracing::error!("could not stop Meka: {error:#}"),
    }
    end
}

/// Asks `child` to stop by closing `input`, its standard input, and kills it if it has not stopped
/// within [`STOP_WAIT`].
fn stop(child: &mut Child, input: Option<ChildStdin>) -> anyhow::Result<ExitStatus> {
    drop(input);
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
            "Meka is missing from the app: {} is not there. Install the app again.",
            binary.display()
        );
    }
    Ok(())
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
        assert_eq!(args, [
            "serve",
            "--bind",
            "127.0.0.1:41000",
            "--exit-on-stdin-close"
        ]);
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
    fn the_entry_follows_the_token_file_and_another_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        let token = dir.path().join("meka-token");
        std::fs::write(
            &config,
            format!(
                "[[serve.tokens]]\ntoken_file = \"/elsewhere/mine\"\nscopes = [\"sessions:r\"]\n\n\
                 [[serve.tokens]]\ntoken_file = \"/moved/meka-config/meka-token\"\n\
                 description = \"{ENTRY}\"\nscopes = [\"sessions:r\"]\n"
            ),
        )
        .unwrap();
        ensure_token_entry(dir.path(), &token).unwrap();
        let parsed: toml_edit::DocumentMut =
            std::fs::read_to_string(&config).unwrap().parse().unwrap();
        let entries = parsed["serve"]["tokens"].as_array_of_tables().unwrap();
        assert_eq!(entries.len(), 2);
        let mine = entries.get(0).unwrap();
        assert_eq!(mine["token_file"].as_str(), Some("/elsewhere/mine"));
        assert_eq!(mine["scopes"].as_array().unwrap().len(), 1);
        let ours = entries.get(1).unwrap();
        assert_eq!(ours["token_file"].as_str(), token.to_str());
        assert_eq!(ours["scopes"].as_array().unwrap().len(), SCOPES.len());
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

    /// A stand-in Meka: a script that serves nothing, notes when its standard input closes, and
    /// ends.
    #[cfg(unix)]
    fn stand_in(dir: &Path) -> Launch {
        use std::os::unix::fs::PermissionsExt;
        let binary = dir.join("meka");
        std::fs::write(
            &binary,
            "#!/bin/sh\necho started >> \"$MEKA_DATA_DIR/runs\"\ncat > /dev/null\necho closed >> \"$MEKA_DATA_DIR/runs\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        Launch {
            binary,
            config_dir: dir.to_path_buf(),
            data_dir: dir.to_path_buf(),
            address: free_address().unwrap(),
            log: dir.join("meka.log"),
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
        wait_for(&runs, "started\nclosed\nstarted\n");
        meka.stop();
        wait_for(&runs, "started\nclosed\nstarted\nclosed\n");
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

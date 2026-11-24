//! Pseudo-terminals on Unix.

use std::path::Path;

use rustix::process::{Pid, Signal};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::Started;

/// What the shell writes.
pub(crate) struct Output(pty_process::OwnedReadPty);

/// Where what is typed goes.
pub(crate) struct Input(pty_process::OwnedWritePty);

/// The shell, to wait for.
pub(crate) struct Shell(tokio::process::Child);

/// The shell's process group, which is its session's and has its id.
pub(crate) struct Process(Option<Pid>);

/// Starts `shell` as a login shell with `args` in `cwd`, on a pseudo-terminal `cols` by `rows`.
pub(crate) fn start(
    shell: &Path,
    args: &[String],
    cwd: &Path,
    cols: u16,
    rows: u16,
) -> anyhow::Result<Started> {
    let (pty, pts) = pty_process::open().map_err(plain)?;
    pty.resize(pty_process::Size::new(rows, cols))
        .map_err(plain)?;
    let name = shell
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sh".to_owned());
    let child = pty_process::Command::new(shell)
        .arg0(format!("-{name}"))
        .args(args)
        .current_dir(cwd)
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .kill_on_drop(true)
        .spawn(pts)
        .map_err(plain)?;
    let pid = child
        .id()
        .and_then(|pid| i32::try_from(pid).ok())
        .and_then(Pid::from_raw);
    let (reader, writer) = pty.into_split();
    Ok(Started {
        output: Output(reader),
        input: Input(writer),
        shell: Shell(child),
        process: Process(pid),
    })
}

/// `error` as its message alone: pty-process's errors repeat their cause in their message, so
/// with the cause they would say it twice.
fn plain(error: pty_process::Error) -> anyhow::Error {
    anyhow::anyhow!("{error}")
}

impl Output {
    /// Reads what the shell wrote into `buffer`; nothing at its end.
    pub(crate) async fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buffer).await
    }
}

impl Input {
    /// Types `data`.
    pub(crate) async fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.0.write_all(data).await?;
        self.0.flush().await
    }

    /// Resizes the terminal.
    pub(crate) fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.0
            .resize(pty_process::Size::new(rows, cols))
            .map_err(plain)
    }

    /// Closes the pseudo-terminal: what still holds the other end reads its end of input and is
    /// sent `SIGHUP`.
    #[expect(
        clippy::unused_async,
        reason = "Windows closes its console off the runtime"
    )]
    pub(crate) async fn close(self) {
        drop(self);
    }
}

impl Shell {
    /// Waits for the shell to end; its exit code, if it had one.
    pub(crate) async fn wait(mut self) -> Option<i32> {
        self.0.wait().await.ok().and_then(|status| status.code())
    }
}

impl Process {
    /// Sends the shell's process group `SIGHUP`, as closing a terminal window does.
    pub(crate) fn hang_up(&self) {
        self.signal(Signal::HUP);
    }

    /// Kills the shell's process group.
    pub(crate) fn kill(&self) {
        self.signal(Signal::KILL);
    }

    fn signal(&self, signal: Signal) {
        if let Some(pid) = self.0
            && let Err(error) = rustix::process::kill_process_group(pid, signal)
        {
            tracing::debug!("could not signal a terminal's shell: {error}");
        }
    }
}

/// The user's login shell, from the user database, else `$SHELL`, else `/bin/sh`.
pub(crate) fn default_shell() -> String {
    database_shell()
        .filter(|shell| Path::new(shell).is_absolute() && Path::new(shell).exists())
        .or_else(|| {
            std::env::var("SHELL")
                .ok()
                .filter(|shell| Path::new(shell).exists())
        })
        .unwrap_or_else(|| "/bin/sh".to_owned())
}

/// The login shell the user database gives the process's user: `/etc/passwd` on Linux, Directory
/// Services on macOS.
fn database_shell() -> Option<String> {
    let uid = rustix::process::getuid().as_raw();
    let mut size = 16 * 1024;
    loop {
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut buffer = vec![0_u8; size];
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: `entry` and `buffer` live through the call, which fills them in and points
        // `found` at `entry`, or leaves it null.
        let code = unsafe {
            libc::getpwuid_r(
                uid,
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &raw mut found,
            )
        };
        if code == libc::ERANGE && size < 1024 * 1024 {
            size *= 4;
            continue;
        }
        if code != 0 || found.is_null() {
            return None;
        }
        // SAFETY: `found` points at `entry`, filled in, whose strings point into `buffer`, which
        // is still here.
        let shell = unsafe { (*found).pw_shell };
        if shell.is_null() {
            return None;
        }
        // SAFETY: a NUL-terminated string in `buffer`.
        let shell = unsafe { std::ffi::CStr::from_ptr(shell) };
        return shell.to_str().ok().map(str::to_owned);
    }
}

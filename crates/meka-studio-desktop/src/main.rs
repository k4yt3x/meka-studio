//! Meka Studio as a desktop app. It runs a Meka of its own, apart from any other on the computer,
//! and Meka Studio's server in its own process on loopback, and shows Meka Studio in its window.
//! Closing the window hides it or quits, as the settings say; quitting stops the server, then
//! Meka, which cancels its turns.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod drops;
mod files;
mod http;
mod meka;
mod shell;
#[cfg(windows)]
mod system;
mod update;
mod window;

use std::{
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use meka_studio::desktop::{Options, ProcessState, Server};
use tauri::{Manager, RunEvent};
use tauri_plugin_notification::NotificationExt;
use tracing_subscriber::EnvFilter;

use crate::{
    files::Paths,
    meka::{Launch, Meka, MekaApi},
    shell::{Opens, Shell},
};

/// How long the app waits for Meka to answer before it starts Meka Studio anyway, which then says
/// Meka cannot be reached, with a way to restart it.
const MEKA_START_WAIT: Duration = Duration::from_secs(30);

/// The ports Meka Studio's server is given one of, at first start: below where systems hand out
/// ports of their own, so the port is likely free at the next start too.
const STUDIO_PORTS: std::ops::Range<u16> = 20_000..30_000;

/// The argument that has a running app quit, and is how its installer quits it before replacing
/// or removing it. Started with it while no copy runs, the app ends at once.
const QUIT: &str = "--quit";

fn main() {
    meka_studio::desktop::prepare();
    let context = tauri::generate_context!();
    // The app's name wherever the system shows it, `productName` in tauri.conf.json.
    let name = context.package_info().name.clone();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(started_again))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            if let Err(error) = setup(app.handle()) {
                let name = &app.package_info().name;
                window::fail_to_start(app.handle(), &format!("{name} could not start: {error:#}"));
            }
            Ok(())
        })
        .build(context);
    let app = match app {
        Ok(app) => app,
        Err(error) => {
            let message = format!("{name} could not start: {error}");
            tracing::error!("{message}");
            // A release build on Windows has no console to write to.
            #[cfg(windows)]
            system::message_box(&name, &message);
            #[cfg(not(windows))]
            eprintln!("{message}");
            std::process::exit(1);
        }
    };
    app.run(|app, event| match event {
        // The event loop ending by itself, as when its last window is gone, stops the server and
        // Meka first.
        RunEvent::ExitRequested { api, .. } if app.try_state::<Arc<Shell>>().is_some() => {
            api.prevent_exit();
            quit(app);
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => window::show_main(app),
        _ => {}
    });
}

/// Quits: hides the windows, stops Meka Studio's server and Meka, and ends.
pub(crate) fn quit(app: &tauri::AppHandle) {
    let Some(shell) = app
        .try_state::<Arc<Shell>>()
        .map(|shell| Arc::clone(&shell))
    else {
        // Only the start page's window is open, saying why the app could not start.
        app.exit(0);
        return;
    };
    if shell.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(window) = app.get_webview_window(window::MAIN) {
        shell.keep_place(&window);
    }
    for window in app.webview_windows().values() {
        if let Err(error) = window.hide() {
            tracing::warn!("could not hide a window: {error}");
        }
    }
    let app = app.clone();
    std::thread::spawn(move || {
        shell.shut_down();
        #[cfg(windows)]
        shell.name_notifications(false);
        // Ended here rather than by asking the event loop: on Windows a loop with every window
        // hidden waits for one more message before it sees the request, which may never come.
        app.cleanup_before_exit();
        std::process::exit(0);
    });
}

/// Quits as a person asks, from the keys, the menus or closing the window: while turns run, first
/// asking whether to stop them. One question shows at a time.
pub(crate) fn ask_to_quit(app: &tauri::AppHandle) {
    let Some(shell) = app
        .try_state::<Arc<Shell>>()
        .map(|shell| Arc::clone(&shell))
    else {
        quit(app);
        return;
    };
    if shell.asking_to_quit.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let running = shell.turns_running().await;
        if running == 0 {
            quit(&app);
            return;
        }
        let answering = app.clone();
        window::ask_to_stop_turns(&app, running, move |quitting| {
            shell.asking_to_quit.store(false, Ordering::SeqCst);
            if quitting {
                quit(&answering);
            }
        });
    });
}

/// What starting the app again, with `arguments` in `folder`, asks of the running app: to quit,
/// with [`QUIT`], or else to show its window, saying where it runs from if the start was of
/// another copy, which then ends.
fn started_again(app: &tauri::AppHandle, arguments: Vec<String>, folder: String) {
    if arguments.iter().skip(1).any(|argument| argument == QUIT) {
        quit(app);
        return;
    }
    window::show_main(app);
    let Some(started) = started_program(&arguments, Path::new(&folder)) else {
        return;
    };
    let Ok(ours) = std::env::current_exe() else {
        return;
    };
    // Unless both programs are found, which copy started is not known, and nothing is said.
    match (
        std::fs::canonicalize(&started),
        std::fs::canonicalize(&ours),
    ) {
        (Ok(started), Ok(canonical)) if started != canonical => {}
        _ => return,
    }
    if let Some(shell) = app.try_state::<Arc<Shell>>() {
        shell.show(
            &format!("{} is already running", app.package_info().name),
            &format!("It runs from {}.", ours.parent().unwrap_or(&ours).display()),
            None,
        );
    }
}

/// The program a start of the app with `arguments` in `folder` ran: its first argument, as the
/// system passed it. A bare name was found on the `PATH`, which is not passed on, so it tells
/// nothing.
fn started_program(arguments: &[String], folder: &Path) -> Option<PathBuf> {
    let program = Path::new(arguments.first()?);
    if program
        .parent()
        .is_none_or(|parent| parent.as_os_str().is_empty())
    {
        return None;
    }
    let mut program = folder.join(program);
    if cfg!(windows) && program.extension().is_none() {
        program.set_extension("exe");
    }
    Some(program)
}

/// Opens the window and the tray icon, then starts Meka and Meka Studio behind them.
fn setup(app: &tauri::AppHandle) -> anyhow::Result<()> {
    if std::env::args_os().skip(1).any(|argument| argument == QUIT) {
        std::process::exit(0);
    }
    let paths = Paths::of(app)?;
    if let Err(error) = paths.make().and_then(|()| logging(&paths)) {
        return Err(match &paths.portable {
            Some(_) => anyhow::anyhow!(
                "{error:#}. A portable copy keeps its data beside it; move it to a folder you can \
                 write to."
            ),
            None => error,
        });
    }
    #[cfg(windows)]
    if paths.portable.is_none() {
        system::hold_running_mutex();
    }
    let shell = Arc::new(Shell::new(app.clone(), paths));
    app.manage(Arc::clone(&shell));
    let listener = studio_listener(&shell)?;
    let origin = format!("http://{}", listener.local_addr()?).parse()?;
    app.on_menu_event(window::chosen);
    #[cfg(target_os = "macos")]
    window::menu_bar(app)?;
    window::open_main(app, &origin)?;
    window::tray(app)?;
    let clicked = app.clone();
    app.notification().on_action(move |action| {
        if action.action_id() != "tap" {
            return;
        }
        if let Some(opens) = action
            .notification()
            .and_then(|notification| Opens::from_extra(notification.extra()))
        {
            opens.open(&clicked);
        }
    })?;
    #[cfg(windows)]
    if shell.settings().notifications {
        shell.name_notifications(true);
    }
    let starting = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = start(&starting, &shell, listener).await {
            let message = format!(
                "{} could not start: {error:#}",
                starting.package_info().name
            );
            tracing::error!("{message}");
            window::show_error(&starting, &message);
        }
    });
    Ok(())
}

/// Starts Meka, then Meka Studio's server once Meka answers, and opens Meka Studio in the window.
async fn start(
    app: &tauri::AppHandle,
    shell: &Arc<Shell>,
    listener: TcpListener,
) -> anyhow::Result<()> {
    let paths = &shell.paths;
    meka::check_binary(&paths.meka)?;
    let token = meka::new_token(&paths.meka_token)?;
    meka::ensure_token_entry(&paths.meka_config, &paths.meka_token)?;
    let launch = Launch {
        binary: paths.meka.clone(),
        config_dir: paths.meka_config.clone(),
        data_dir: paths.meka_data.clone(),
        address: meka::free_address()?,
        log: paths.logs.join("meka.log"),
        home: app.path().home_dir()?,
    };
    let api = MekaApi {
        url: format!("http://{}", launch.address).parse()?,
        token,
    };
    let meka = Arc::new(Meka::start(launch)?);
    shell.keep_meka(Arc::clone(&meka), api.clone());
    // The server starts once Meka answers, so its first look at Meka finds it there.
    let mut process = meka.watch();
    if tokio::time::timeout(
        MEKA_START_WAIT,
        process.wait_for(|process| process.state == ProcessState::Running),
    )
    .await
    .is_err()
    {
        tracing::warn!("Meka did not answer within {MEKA_START_WAIT:?}");
    }
    let server = Server::open(Options {
        listener: tokio::net::TcpListener::from_std(listener)?,
        state_dir: paths.studio.clone(),
        meka_url: api.url,
        meka_token: api.token,
        host: Arc::clone(shell) as Arc<dyn meka_studio::desktop::Host>,
    })
    .await?;
    let entry = server.entry();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tauri::async_runtime::spawn(async move {
        let stopped = async {
            // A dropped sender stops the server as well.
            let _ = stopped.await;
        };
        if let Err(error) = server.serve(stopped).await {
            tracing::error!("Meka Studio's server stopped: {error:#}");
        }
    });
    shell.keep_server(stop, task);
    window::enter(app, &entry)?;
    tauri::async_runtime::spawn(update::follow(Arc::clone(shell)));
    Ok(())
}

/// The loopback port Meka Studio's server listens on: the one it had before, which the window's
/// storage belongs to, or a new one if that is taken or there was none.
fn studio_listener(shell: &Shell) -> anyhow::Result<TcpListener> {
    let kept = shell.memory().port;
    let listener = match kept.and_then(|port| TcpListener::bind(("127.0.0.1", port)).ok()) {
        Some(listener) => listener,
        None => {
            if let Some(port) = kept {
                tracing::warn!(
                    "port {port} is taken, so Meka Studio moves to another, and the window's \
                     appearance settings start afresh"
                );
            }
            new_listener()?
        }
    };
    let port = listener.local_addr()?.port();
    if kept != Some(port) {
        shell.remember(|memory| memory.port = Some(port));
    }
    listener.set_nonblocking(true)?;
    Ok(listener)
}

/// A listener on a free port among [`STUDIO_PORTS`], or on any free port.
fn new_listener() -> anyhow::Result<TcpListener> {
    let span = STUDIO_PORTS.end - STUDIO_PORTS.start;
    for _ in 0..64 {
        let mut bytes = [0_u8; 2];
        getrandom::fill(&mut bytes).map_err(|error| anyhow::anyhow!("no randomness: {error}"))?;
        let port = STUDIO_PORTS.start + u16::from_ne_bytes(bytes) % span;
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return Ok(listener);
        }
    }
    Ok(TcpListener::bind("127.0.0.1:0")?)
}

/// Logs to `meka-studio.log` in the logs' folder, or in a debug build to standard error.
fn logging(paths: &Paths) -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(meka_studio::DEFAULT_LOG));
    let subscriber = tracing_subscriber::fmt().with_env_filter(filter);
    let set = if cfg!(debug_assertions) {
        subscriber.with_writer(std::io::stderr).try_init()
    } else {
        let log = files::open_log(&paths.logs.join("meka-studio.log"))?;
        subscriber
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(log))
            .try_init()
    };
    set.map_err(|error| anyhow::anyhow!("could not start logging: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_names_its_program_unless_it_was_found_on_the_path() {
        let folder = Path::new("/home/user/Downloads");
        let started = |first: &str| started_program(&[first.to_owned(), QUIT.to_owned()], folder);
        let program = |path: &str| {
            let mut path = PathBuf::from(path);
            if cfg!(windows) {
                path.set_extension("exe");
            }
            Some(path)
        };
        assert_eq!(
            started("/opt/meka-studio/meka-studio-desktop"),
            program("/opt/meka-studio/meka-studio-desktop")
        );
        assert_eq!(
            started("copy/meka-studio-desktop"),
            program("/home/user/Downloads/copy/meka-studio-desktop")
        );
        assert_eq!(started("meka-studio-desktop"), None);
        assert_eq!(started_program(&[], folder), None);
    }
}

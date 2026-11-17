//! Meka Studio as a desktop app. It runs a Meka of its own, apart from any other on the computer,
//! and Meka Studio's server in its own process on loopback, and shows Meka Studio in its window.
//! Closing the window hides it or quits, as the settings say; quitting stops the server, then
//! Meka, which cancels its turns.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod files;
mod meka;
mod shell;
mod update;
mod window;

use std::{
    net::TcpListener,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use meka_studio::desktop::{Options, ProcessState, Server};
use tauri::{Manager, RunEvent};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::{AppHandleExt, StateFlags};
use tracing_subscriber::EnvFilter;

use crate::{
    files::Paths,
    meka::{Launch, Meka},
    shell::Shell,
};

/// How long the app waits for Meka to answer before it starts Meka Studio anyway, which then says
/// Meka cannot be reached, with a way to restart it.
const MEKA_START_WAIT: Duration = Duration::from_secs(30);

/// The ports Meka Studio's server is given one of, at first start: below where systems hand out
/// ports of their own, so the port is likely free at the next start too.
const STUDIO_PORTS: std::ops::Range<u16> = 20_000..30_000;

/// What the window keeps of itself from one start to the next: everything but whether it is
/// shown and whether the system draws its title bar, which the app decides.
fn kept_state() -> StateFlags {
    StateFlags::all() - StateFlags::VISIBLE - StateFlags::DECORATIONS
}

fn main() {
    meka_studio::desktop::prepare();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(
            |app, _arguments, _folder| {
                window::show_main(app);
            },
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(kept_state())
                .with_filter(|label| label == window::MAIN)
                .build(),
        )
        .setup(|app| Ok(setup(app.handle())?))
        .build(tauri::generate_context!());
    let app = match app {
        Ok(app) => app,
        Err(error) => {
            tracing::error!("Meka Studio could not start: {error}");
            eprintln!("Meka Studio could not start: {error}");
            std::process::exit(1);
        }
    };
    app.run(|app, event| match event {
        // Quitting from the system, such as Cmd+Q on macOS, quits as the app's own Quit does.
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
        return;
    };
    if shell.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Err(error) = app.save_window_state(kept_state()) {
        tracing::warn!("could not keep the window's size and place: {error}");
    }
    for window in app.webview_windows().values() {
        if let Err(error) = window.hide() {
            tracing::warn!("could not hide a window: {error}");
        }
    }
    let app = app.clone();
    std::thread::spawn(move || {
        shell.shut_down();
        // Ended here rather than by asking the event loop: on Windows a loop with every window
        // hidden waits for one more message before it sees the request, which may never come.
        app.cleanup_before_exit();
        std::process::exit(0);
    });
}

/// Opens the window and the tray icon, then starts Meka and Meka Studio behind them.
fn setup(app: &tauri::AppHandle) -> anyhow::Result<()> {
    let paths = Paths::of(app)?;
    paths.make()?;
    logging(&paths)?;
    let shell = Arc::new(Shell::new(app.clone(), paths));
    app.manage(Arc::clone(&shell));
    let listener = studio_listener(&shell)?;
    let origin = format!("http://{}", listener.local_addr()?).parse()?;
    window::open_main(app, &origin)?;
    window::tray(app)?;
    let clicked = app.clone();
    app.notification().on_action(move |action| {
        if action.action_id() != "tap" {
            return;
        }
        let Some(extra) = action
            .notification()
            .map(|notification| notification.extra())
        else {
            return;
        };
        let text = |name: &str| extra.get(name).and_then(serde_json::Value::as_str);
        if let Some(session) = text("session") {
            window::open_session(&clicked, session);
        } else if let Some(page) = text("page").and_then(|page| page.parse().ok()) {
            window::open_outside(&clicked, &page);
        } else if let Some(file) = text("file")
            && let Err(error) = clicked.opener().reveal_item_in_dir(file)
        {
            tracing::warn!("could not show the download in its folder: {error}");
        }
    })?;
    let starting = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = start(&starting, &shell, listener).await {
            tracing::error!("Meka Studio could not start: {error:#}");
            window::show_error(
                &starting,
                &format!("Meka Studio could not start: {error:#}"),
            );
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
        pid_file: paths.meka_pid.clone(),
        home: app.path().home_dir()?,
    };
    #[cfg(target_os = "macos")]
    tokio::task::spawn_blocking({
        let launch = launch.clone();
        move || meka::stop_leftover(&launch)
    })
    .await?;
    let meka = Arc::new(Meka::start(launch.clone())?);
    shell.keep_meka(Arc::clone(&meka));
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
        meka_url: format!("http://{}", launch.address).parse()?,
        meka_token: token,
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
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
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

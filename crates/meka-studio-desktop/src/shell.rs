//! What the app holds while it runs, and what it does for Meka Studio: system notifications, its
//! settings, restarting Meka, and opening the logs' folder.

use std::{
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicI32, Ordering},
    },
    time::Duration,
};

use meka_studio::desktop::{
    DesktopSettings, DesktopState, Host, MekaProcess, Notification, ProcessState, Update,
};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

use crate::{
    files::{self, Memory, Paths},
    meka::Meka,
};

/// How long Meka Studio's server has to stop, closing its streams and terminals.
const SERVER_STOP_WAIT: Duration = Duration::from_secs(15);

/// What a notification opens when clicked.
pub(crate) enum Opens {
    /// A Meka session, in the window.
    Session(String),
    /// A web page, in the browser.
    Page(String),
    /// A file, shown in its folder.
    File(String),
}

impl Opens {
    /// The name and value a notification carries this as.
    fn extra(&self) -> (&'static str, &str) {
        match self {
            Self::Session(session) => ("session", session),
            Self::Page(url) => ("page", url),
            Self::File(path) => ("file", path),
        }
    }
}

/// The running app.
pub(crate) struct Shell {
    pub(crate) app: AppHandle,
    pub(crate) paths: Paths,
    settings: Mutex<DesktopSettings>,
    memory: Mutex<Memory>,
    update: Mutex<Option<Update>>,
    meka: OnceLock<Arc<Meka>>,
    /// Stops Meka Studio's server, and the task it runs on.
    server: Mutex<
        Option<(
            tokio::sync::oneshot::Sender<()>,
            tauri::async_runtime::JoinHandle<()>,
        )>,
    >,
    /// Told when the settings change.
    pub(crate) settings_changed: tokio::sync::Notify,
    /// Whether the app is quitting.
    pub(crate) quitting: AtomicBool,
    last_notification: AtomicI32,
}

impl Shell {
    /// The app with its folders at `paths`, its settings and memory read from them.
    pub(crate) fn new(app: AppHandle, paths: Paths) -> Self {
        let settings = files::read_settings(&paths.settings);
        let memory = files::read_memory(&paths.memory);
        Self {
            app,
            paths,
            settings: Mutex::new(settings),
            memory: Mutex::new(memory),
            update: Mutex::new(None),
            meka: OnceLock::new(),
            server: Mutex::new(None),
            settings_changed: tokio::sync::Notify::new(),
            quitting: AtomicBool::new(false),
            last_notification: AtomicI32::new(0),
        }
    }

    /// The app's settings.
    pub(crate) fn settings(&self) -> DesktopSettings {
        *lock(&self.settings)
    }

    /// What the app remembers.
    pub(crate) fn memory(&self) -> Memory {
        lock(&self.memory).clone()
    }

    /// Changes what the app remembers, and writes it down.
    pub(crate) fn remember(&self, change: impl FnOnce(&mut Memory)) {
        let mut memory = lock(&self.memory);
        change(&mut memory);
        if let Err(error) = files::write_json(&self.paths.memory, &*memory) {
            tracing::warn!("could not write what the app remembers: {error:#}");
        }
    }

    /// Keeps the Meka the app runs.
    pub(crate) fn keep_meka(&self, meka: Arc<Meka>) {
        if self.meka.set(meka).is_err() {
            tracing::error!("Meka was started twice");
        }
    }

    /// Keeps the way to stop Meka Studio's server.
    pub(crate) fn keep_server(
        &self,
        stop: tokio::sync::oneshot::Sender<()>,
        task: tauri::async_runtime::JoinHandle<()>,
    ) {
        *lock(&self.server) = Some((stop, task));
    }

    /// Notes the newest release, or that this one is the newest, telling of a newer one once.
    pub(crate) fn found_release(&self, update: Option<Update>) {
        if let Some(update) = &update
            && self.memory().told_update.as_ref() != Some(&update.version)
        {
            self.show(
                &format!("Meka Studio {} is available", update.version),
                "Click to open its release page.",
                Some(Opens::Page(update.url.clone())),
            );
            let version = update.version.clone();
            self.remember(|memory| memory.told_update = Some(version));
        }
        *lock(&self.update) = update;
    }

    /// Shows a system notification, which opens `opens` when clicked, if the settings show them.
    pub(crate) fn show(&self, title: &str, body: &str, opens: Option<Opens>) {
        if !self.settings().notifications {
            return;
        }
        let id = self.last_notification.fetch_add(1, Ordering::Relaxed) + 1;
        let mut builder = self
            .app
            .notification()
            .builder()
            .id(id)
            .title(title)
            .body(body);
        if let Some(opens) = &opens {
            let (name, value) = opens.extra();
            builder = builder.extra(name, value);
        }
        if let Err(error) = builder.show() {
            tracing::warn!("could not show a notification: {error}");
        }
    }

    /// Stops Meka Studio's server, then Meka, waiting for both.
    pub(crate) fn shut_down(&self) {
        let server = lock(&self.server).take();
        if let Some((stop, task)) = server {
            // The server may have stopped already, and dropped its end.
            let _ = stop.send(());
            tauri::async_runtime::block_on(async {
                if tokio::time::timeout(SERVER_STOP_WAIT, task).await.is_err() {
                    tracing::warn!("Meka Studio's server did not stop in time");
                }
            });
        }
        if let Some(meka) = self.meka.get() {
            meka.stop();
        }
    }
}

impl Host for Shell {
    fn notify(&self, notification: Notification) {
        self.show(
            &notification.title,
            &notification.body,
            notification.session.map(Opens::Session),
        );
    }

    fn state(&self) -> DesktopState {
        DesktopState {
            settings: self.settings(),
            meka: self.meka.get().map_or(
                MekaProcess {
                    state: ProcessState::Starting,
                    pid: None,
                    last_exit: None,
                },
                |meka| meka.state(),
            ),
            update: lock(&self.update).clone(),
            logs: self.paths.logs.display().to_string(),
        }
    }

    fn save_settings(&self, settings: DesktopSettings) -> anyhow::Result<()> {
        // Held across the write, so saves one after another leave the file and the app agreeing.
        let mut current = lock(&self.settings);
        files::write_json(&self.paths.settings, &settings)?;
        *current = settings;
        drop(current);
        self.settings_changed.notify_one();
        Ok(())
    }

    fn restart_meka(&self) {
        match self.meka.get() {
            Some(meka) => meka.restart(),
            None => tracing::warn!("Meka is not started yet, so it is not restarted"),
        }
    }

    fn open_logs(&self) -> anyhow::Result<()> {
        self.app
            .opener()
            .open_path(self.paths.logs.to_string_lossy(), None::<&str>)?;
        Ok(())
    }
}

/// `mutex`, whatever a thread that panicked holding it left.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

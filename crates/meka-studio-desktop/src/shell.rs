//! What the app holds while it runs, and what it does for Meka Studio: system notifications, its
//! settings, restarting Meka, and opening its folders. It keeps the main window's place too.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicI32, Ordering},
    },
    time::Duration,
};

use meka_studio::desktop::{
    DesktopSettings, DesktopState, Folder, Host, MekaProcess, Notification, ProcessState, Update,
};
use tauri::{AppHandle, WebviewWindow};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

use crate::{
    files::{self, Memory, Paths, WindowPlace},
    meka::Meka,
    window,
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
    const FILE: &'static str = "file";
    const PAGE: &'static str = "page";
    const SESSION: &'static str = "session";

    /// The name and value a notification carries this as, which [`Self::from_extra`] reads.
    fn extra(&self) -> (&'static str, &str) {
        match self {
            Self::Session(session) => (Self::SESSION, session),
            Self::Page(url) => (Self::PAGE, url),
            Self::File(path) => (Self::FILE, path),
        }
    }

    /// What a notification carrying `extra` opens, as [`Self::extra`] wrote it.
    pub(crate) fn from_extra(extra: &HashMap<String, serde_json::Value>) -> Option<Self> {
        let text = |name: &str| Some(extra.get(name)?.as_str()?.to_owned());
        text(Self::SESSION)
            .map(Self::Session)
            .or_else(|| text(Self::PAGE).map(Self::Page))
            .or_else(|| text(Self::FILE).map(Self::File))
    }

    /// Opens it: a session in the main window, a page in the browser, a file in its folder.
    pub(crate) fn open(self, app: &AppHandle) {
        match self {
            Self::Session(session) => window::open_session(app, &session),
            Self::Page(url) => match url.parse() {
                Ok(url) => window::open_outside(app, &url),
                Err(error) => tracing::warn!("not opening {url}: {error}"),
            },
            Self::File(path) => {
                if let Err(error) = app.opener().reveal_item_in_dir(path) {
                    tracing::warn!("could not show the download in its folder: {error}");
                }
            }
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
    /// Where the main window was when last neither maximized nor minimized.
    place: Mutex<Option<WindowPlace>>,
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
            place: Mutex::new(None),
        }
    }

    /// Notes where the main window `window` is, unless it is maximized or minimized, as it `moved`
    /// or resized.
    pub(crate) fn note_place(&self, window: &WebviewWindow, moved: bool) {
        match window::normal_place(window) {
            Ok(Some(place)) => {
                let mut noted = lock(&self.place);
                *noted = place_to_note(*noted, place, moved);
            }
            Ok(None) => {}
            Err(error) => tracing::warn!("could not tell where the window is: {error}"),
        }
    }

    /// Writes down where the main window `window` is, and whether it is maximized, to open it
    /// there at the next start.
    pub(crate) fn keep_place(&self, window: &WebviewWindow) {
        self.note_place(window, false);
        let maximized = match window.is_maximized() {
            Ok(maximized) => maximized,
            Err(error) => {
                tracing::warn!("could not tell whether the window is maximized: {error}");
                return;
            }
        };
        let Some(place) = *lock(&self.place) else {
            return;
        };
        let place = WindowPlace { maximized, ..place };
        if self.memory().window != Some(place) {
            self.remember(|memory| memory.window = Some(place));
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
                &format!(
                    "{} {} is available",
                    self.app.package_info().name,
                    update.version
                ),
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

    /// Registers the name the app's notifications show under, in a portable copy, or with `shown`
    /// false, removes it; see [`crate::system`].
    #[cfg(windows)]
    pub(crate) fn name_notifications(&self, shown: bool) {
        /// The icon notifications show, which Windows reads from a file.
        const ICON: &[u8] = include_bytes!("../icons/128x128.png");
        let Some(root) = &self.paths.portable else {
            return;
        };
        let app_id = &self.app.config().identifier;
        let named = if shown {
            let icon = root.join("icon.png");
            // Written again when a newer version's differs.
            let written = if std::fs::read(&icon).is_ok_and(|kept| kept == ICON) {
                Ok(())
            } else {
                files::write_whole(&icon, ICON)
            };
            written.and_then(|()| {
                crate::system::register_app_id(app_id, &self.app.package_info().name, &icon, root)
            })
        } else {
            crate::system::unregister_app_id(app_id)
        };
        if let Err(error) = named {
            tracing::warn!("could not register the name notifications show under: {error:#}");
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
            portable: self.paths.portable.is_some(),
        }
    }

    fn save_settings(&self, settings: DesktopSettings) -> anyhow::Result<()> {
        // Held across the write, so saves one after another leave the file and the app agreeing.
        let mut current = lock(&self.settings);
        files::write_json(&self.paths.settings, &settings)?;
        #[cfg(windows)]
        if settings.notifications != current.notifications {
            self.name_notifications(settings.notifications);
        }
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

    fn open_folder(&self, folder: Folder) -> anyhow::Result<()> {
        let path = match folder {
            Folder::Config => &self.paths.meka_config,
            Folder::Data => &self.paths.meka_data,
            Folder::Logs => &self.paths.logs,
        };
        self.app
            .opener()
            .open_path(path.to_string_lossy(), None::<&str>)?;
        Ok(())
    }
}

/// The window's place to note, of the one `noted` and the place `now` it `moved` or resized to. A
/// move that changes the window's size starts maximizing it, on Windows before the window knows it
/// is maximized, so it leaves the noted place as it is; the resize that follows decides.
fn place_to_note(noted: Option<WindowPlace>, now: WindowPlace, moved: bool) -> Option<WindowPlace> {
    match noted {
        Some(noted) if moved && (noted.width, noted.height) != (now.width, now.height) => {
            Some(noted)
        }
        _ => Some(now),
    }
}

/// `mutex`, whatever a thread that panicked holding it left.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clicked_notification_opens_what_it_was_shown_with() {
        let carried = |opens: &Opens| {
            let (name, value) = opens.extra();
            HashMap::from([(name.to_owned(), serde_json::Value::from(value))])
        };
        for opens in [
            Opens::Session("s1".to_owned()),
            Opens::Page("https://example.org/".to_owned()),
            Opens::File("/home/me/Downloads/a.txt".to_owned()),
        ] {
            let read = Opens::from_extra(&carried(&opens));
            assert_eq!(read.as_ref().map(Opens::extra), Some(opens.extra()));
        }
        assert!(Opens::from_extra(&HashMap::new()).is_none());
    }

    #[test]
    fn a_move_that_resizes_the_window_leaves_its_place_to_the_resize() {
        let place = |x, width| WindowPlace {
            x,
            y: 40,
            width,
            height: 720,
            maximized: false,
        };
        let noted = Some(place(300, 1100));
        assert_eq!(
            place_to_note(noted, place(500, 1100), true),
            Some(place(500, 1100))
        );
        assert_eq!(place_to_note(noted, place(-8, 1936), true), noted);
        assert_eq!(
            place_to_note(noted, place(-8, 1936), false),
            Some(place(-8, 1936))
        );
        assert_eq!(
            place_to_note(None, place(-8, 1936), true),
            Some(place(-8, 1936))
        );
    }
}

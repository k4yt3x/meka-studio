//! Meka Studio inside the desktop app. The app runs Meka, and this server on loopback in its own
//! process; its window opens the interface at [`Server::entry`], which signs it in once. What only
//! the app can do (system notifications, its settings, restarting Meka) the server
//! asks of it through [`Host`], and the interface reaches through `/api/desktop`.

use std::{
    future::Future,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use url::Url;
use utoipa::ToSchema;

use crate::{
    app::{self, App},
    auth::sha256_hex,
    config::Config,
    store::Store,
};

/// What the desktop app does for Meka Studio.
pub trait Host: Send + Sync + 'static {
    /// Shows `notification` as a system notification, if the app's settings show them.
    fn notify(&self, notification: Notification);
    /// How the app stands.
    fn state(&self) -> DesktopState;
    /// Saves `settings`, which apply from then on.
    fn save_settings(&self, settings: DesktopSettings) -> anyhow::Result<()>;
    /// Stops Meka and starts it again.
    fn restart_meka(&self);
    /// Opens `folder` in the system's file manager.
    fn open_folder(&self, folder: Folder) -> anyhow::Result<()>;
}

/// A folder of the desktop app's that the interface can open in the file manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Folder {
    /// Meka's config folder, with its `config.toml`.
    Config,
    /// Meka's data folder, with its sessions.
    Data,
    /// The logs' folder: the app's log and Meka's.
    Logs,
}

/// A system notification: the session's title, what happened in it, and the session, which a
/// click opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub title: String,
    pub body: String,
    pub session: Option<String>,
}

/// How the desktop app stands.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DesktopState {
    /// The app's settings.
    pub settings: DesktopSettings,
    /// The Meka the app runs.
    pub meka: MekaProcess,
    /// A newer release of Meka Studio, if the last check found one.
    pub update: Option<Update>,
    /// The folder Meka's log is written to.
    pub logs: String,
    /// Whether the app is a portable copy, keeping everything in a folder beside it.
    pub portable: bool,
}

/// The desktop app's settings. One missing from the saved settings takes its default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct DesktopSettings {
    /// What closing the window does.
    #[schema(required = true)]
    pub close: CloseAction,
    /// Whether system notifications are shown: of turns, downloads and newer releases.
    #[schema(required = true)]
    pub notifications: bool,
    /// Whether GitHub is asked once a day for a newer release.
    #[schema(required = true)]
    pub check_updates: bool,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            // A closed window keeps the app running on macOS, as its apps do; elsewhere the first
            // close asks.
            close: if cfg!(target_os = "macos") {
                CloseAction::Tray
            } else {
                CloseAction::Ask
            },
            notifications: true,
            check_updates: true,
        }
    }
}

/// What closing the window does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CloseAction {
    /// Asks whether to keep running in the tray or quit, and keeps the answer.
    Ask,
    /// Hides the window; Meka and its turns keep running, and the tray icon shows it again.
    Tray,
    /// Quits, stopping Meka and what runs in it.
    Quit,
}

/// The Meka the desktop app runs.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MekaProcess {
    /// How its process stands.
    pub state: ProcessState,
    /// Its process id, while it runs.
    pub pid: Option<u32>,
    /// How it last ended, such as "exit status: 1", if it has.
    pub last_exit: Option<String>,
}

/// How Meka's process stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProcessState {
    /// Started, and not answering yet.
    Starting,
    /// Answering.
    Running,
    /// Stopping or ended, and about to start again.
    Restarting,
    /// Stopped.
    Stopped,
}

/// A newer release of Meka Studio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Update {
    /// Its version.
    pub version: String,
    /// Its release page.
    pub url: String,
}

/// The desktop app, as the server holds it: the app, and the sign-in its window holds.
#[derive(Clone)]
pub(crate) struct Desktop {
    pub(crate) host: Arc<dyn Host>,
    /// The window's sign-in id, as push notifications track devices by.
    pub(crate) device: String,
    /// The window's way in, until it is taken.
    pub(crate) entry: Arc<Mutex<Option<Entry>>>,
}

/// The window's way in: the ticket the app opens it with, and the sign-in token it is given for it.
pub(crate) struct Entry {
    pub(crate) ticket: String,
    pub(crate) token: String,
}

impl Desktop {
    /// The sign-in token `ticket` is for, once: the ticket is used up by taking it.
    pub(crate) fn redeem(&self, ticket: &str) -> Option<String> {
        // Compared as digests, so how long the comparison takes says nothing of the ticket.
        let offered = sha256_hex(ticket.as_bytes());
        let mut entry = self
            .entry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let matches = entry
            .as_ref()
            .is_some_and(|entry| sha256_hex(entry.ticket.as_bytes()) == offered);
        let token = if matches { entry.take() } else { None };
        drop(entry);
        token.map(|entry| entry.token)
    }
}

/// What a desktop app's server needs.
pub struct Options {
    /// The loopback port it serves on, bound already. The window's storage belongs to this
    /// address, so the app keeps the port from one start to the next.
    pub listener: tokio::net::TcpListener,
    /// Where it keeps its database.
    pub state_dir: PathBuf,
    /// The address of the Meka the app runs.
    pub meka_url: Url,
    /// The token that Meka takes.
    pub meka_token: String,
    /// The app.
    pub host: Arc<dyn Host>,
}

/// Meka Studio's server, as the desktop app runs it.
pub struct Server {
    app: App,
    listener: tokio::net::TcpListener,
    ticket: String,
}

impl Server {
    /// The server `options` describe, with its window's sign-in made and every earlier one ended.
    pub async fn open(options: Options) -> anyhow::Result<Self> {
        let listen = options.listener.local_addr()?;
        let config = Config::desktop(
            listen,
            options.state_dir,
            options.meka_url,
            options.meka_token,
        )?;
        let store = Store::open_in(&config.state_dir)?;
        let (app, ticket) = App::open_desktop(config, store, options.host).await?;
        Ok(Self {
            app,
            listener: options.listener,
            ticket,
        })
    }

    /// The server's address, such as `http://127.0.0.1:41234`.
    fn origin(&self) -> String {
        self.app.config.origin()
    }

    /// Where the window opens the interface: an address that signs it in, once, and goes on to the
    /// interface. Opened again, it goes on without signing in.
    pub fn entry(&self) -> String {
        format!("{}/api/desktop/enter?ticket={}", self.origin(), self.ticket)
    }

    /// Serves until `stop` completes, then ends the connections held open and the terminals.
    pub async fn serve(
        self,
        stop: impl Future<Output = ()> + Send + 'static,
    ) -> anyhow::Result<()> {
        app::serve_on(self.app, self.listener, stop).await
    }
}

/// Prepares the process for Meka Studio; called before any thread starts. On macOS it reads the
/// mode new files are given.
pub fn prepare() {
    crate::files::prepare();
}

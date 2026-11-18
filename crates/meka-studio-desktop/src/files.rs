//! Where the app keeps what it keeps, and the small files it keeps there: its settings, and what
//! it remembers from one start to the next. Meka's config and store are the app's own, apart from
//! any other Meka on the computer. An installed app keeps them in folders of its own in the user's
//! config and data folders; a portable copy, on Windows, in a `data` folder beside it.

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Context;
use meka_studio::desktop::DesktopSettings;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tauri::{AppHandle, Manager};

/// The largest a log grows to before it is set aside, at the next start, for a new one.
const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;

/// The file beside the program that makes it a portable copy. What it holds is not read.
const PORTABLE_MARKER: &str = ".portable";

/// The name of an installed app's folders in the user's config and data folders: as Windows lists
/// the app, and as XDG names folders on Linux. It is not the `productName`, so that renaming the
/// app leaves its data where it is. macOS names the folders after the app's identifier, as Apple
/// asks.
#[cfg(windows)]
const FOLDER: &str = "Meka Studio Desktop";
#[cfg(all(unix, not(target_os = "macos")))]
const FOLDER: &str = "meka-studio-desktop";

/// The name of the webview's data folder, in the folder the app keeps its local data in.
const WEBVIEW: &str = "webview";

/// The app's folders and files.
#[derive(Debug, Clone)]
pub(crate) struct Paths {
    /// Meka's config folder, its `MEKA_CONFIG_DIR`.
    pub(crate) meka_config: PathBuf,
    /// Meka's data folder, its `MEKA_DATA_DIR`.
    pub(crate) meka_data: PathBuf,
    /// The file holding the token Meka takes from the app, in Meka's config folder, which Meka
    /// hides from what the agent runs.
    pub(crate) meka_token: PathBuf,
    /// The Meka binary the app was built with.
    pub(crate) meka: PathBuf,
    /// Meka Studio's state folder.
    pub(crate) studio: PathBuf,
    /// The app's settings.
    pub(crate) settings: PathBuf,
    /// What the app remembers from one start to the next.
    pub(crate) memory: PathBuf,
    /// The logs' folder.
    pub(crate) logs: PathBuf,
    /// The webview's data folder; `None` on macOS, whose webview keeps its data in the user's
    /// Library whatever the app asks.
    pub(crate) webview: Option<PathBuf>,
    /// The folder a portable copy keeps everything in; `None` when the app is installed.
    pub(crate) portable: Option<PathBuf>,
}

impl Paths {
    /// The folder beside the program for a portable copy, or else the app's folders in the user's
    /// config and data folders.
    pub(crate) fn of(app: &AppHandle) -> anyhow::Result<Self> {
        let path = app.path();
        let meka =
            plain(path.resource_dir()?).join(format!("meka{}", std::env::consts::EXE_SUFFIX));
        if let Some(root) = portable_root(&plain(std::env::current_exe()?)) {
            return Ok(Self::portable(root, meka));
        }
        #[cfg(target_os = "macos")]
        let paths = Self::new(
            &path.app_config_dir()?,
            &path.app_data_dir()?,
            path.app_log_dir()?,
            None,
            meka,
            None,
        );
        #[cfg(not(target_os = "macos"))]
        let paths = {
            let local = local_folder(app)?;
            Self::new(
                &path.config_dir()?.join(FOLDER),
                &path.data_dir()?.join(FOLDER),
                local.join("logs"),
                Some(local.join(WEBVIEW)),
                meka,
                None,
            )
        };
        Ok(paths)
    }

    /// A portable copy's folders and files, all in `root`.
    fn portable(root: PathBuf, meka: PathBuf) -> Self {
        Self::new(
            &root,
            &root,
            root.join("logs"),
            Some(root.join(WEBVIEW)),
            meka,
            Some(root.clone()),
        )
    }

    fn new(
        config: &Path,
        data: &Path,
        logs: PathBuf,
        webview: Option<PathBuf>,
        meka: PathBuf,
        portable: Option<PathBuf>,
    ) -> Self {
        let meka_config = config.join("meka-config");
        Self {
            meka_token: meka_config.join("meka-token"),
            meka_config,
            meka_data: data.join("meka-data"),
            meka,
            studio: data.join("studio"),
            settings: config.join("settings.json"),
            memory: data.join("memory.json"),
            logs,
            webview,
            portable,
        }
    }

    /// Makes the folders that are not there yet, readable by their owner alone.
    pub(crate) fn make(&self) -> anyhow::Result<()> {
        for folder in [&self.meka_config, &self.meka_data, &self.studio, &self.logs]
            .into_iter()
            .chain(self.webview.as_ref())
        {
            make_folder(folder)?;
        }
        Ok(())
    }
}

/// The folder an installed app keeps its local data in: its logs, and the webview's data.
#[cfg(not(target_os = "macos"))]
fn local_folder(app: &AppHandle) -> tauri::Result<PathBuf> {
    Ok(app.path().local_data_dir()?.join(FOLDER))
}

/// An installed app's webview data folder, for a window opened before the app has its folders.
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn installed_webview(app: &AppHandle) -> tauri::Result<PathBuf> {
    Ok(local_folder(app)?.join(WEBVIEW))
}

/// The folder a portable copy keeps everything in: `data` beside the program at `exe`, when the
/// [`PORTABLE_MARKER`] file is beside it too, on Windows. Linux keeps an app's folders where the
/// XDG variables say, and macOS keeps a webview's data in the user's Library whatever the app
/// asks, so neither has portable copies.
fn portable_root(exe: &Path) -> Option<PathBuf> {
    let folder = exe.parent()?;
    (cfg!(windows) && folder.join(PORTABLE_MARKER).is_file()).then(|| folder.join("data"))
}

/// `path` without the `\\?\` that Windows puts before a drive in a path read off the running
/// program, which means nothing to a person reading it.
fn plain(path: PathBuf) -> PathBuf {
    if cfg!(windows)
        && let Some(rest) = path.to_str().and_then(|path| path.strip_prefix(r"\\?\"))
        && rest.as_bytes().get(1) == Some(&b':')
    {
        return PathBuf::from(rest);
    }
    path
}

/// Makes `folder` and those above it, readable by their owner alone.
fn make_folder(folder: &Path) -> anyhow::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(folder)
        .with_context(|| format!("could not make {}", folder.display()))
}

/// What the app remembers from one start to the next.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Memory {
    /// The port Meka Studio listened on, which the window's storage belongs to.
    pub(crate) port: Option<u16>,
    /// The newest release the app told of, so it is told of once.
    pub(crate) told_update: Option<String>,
    /// Where the main window was, to open it there again.
    pub(crate) window: Option<WindowPlace>,
}

/// The main window's place on the screen, in physical pixels: its outer edges when it was last
/// neither maximized nor minimized, and whether it was maximized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WindowPlace {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) maximized: bool,
}

/// The app's settings in `path`; their defaults if there are none yet, or if they cannot be read,
/// which is logged.
pub(crate) fn read_settings(path: &Path) -> DesktopSettings {
    read_or_default(path)
}

/// What the app remembers, in `path`; nothing if there is nothing yet, or if it cannot be read,
/// which is logged.
pub(crate) fn read_memory(path: &Path) -> Memory {
    read_or_default(path)
}

fn read_or_default<T: DeserializeOwned + Default>(path: &Path) -> T {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return T::default(),
        Err(error) => {
            tracing::warn!("could not read {}: {error}", path.display());
            return T::default();
        }
    };
    serde_json::from_str(&text).unwrap_or_else(|error| {
        tracing::warn!(
            "{} is not valid, so its defaults hold: {error}",
            path.display()
        );
        T::default()
    })
}

/// The log at `path` to add to, set aside for a new one first when it has grown past
/// [`MAX_LOG_BYTES`].
pub(crate) fn open_log(path: &Path) -> anyhow::Result<std::fs::File> {
    if std::fs::metadata(path).is_ok_and(|metadata| metadata.len() > MAX_LOG_BYTES) {
        std::fs::rename(path, path.with_extension("log.1"))?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("could not open {}", path.display()))
}

/// Writes `value` to `path` as JSON, whole or not at all.
pub(crate) fn write_json(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    let mut text = serde_json::to_vec_pretty(value)?;
    text.push(b'\n');
    write_whole(path, &text)
}

/// Writes `bytes` to `path` through a file beside it, readable by its owner alone, renamed into
/// place, so `path` holds what it held or all of `bytes`.
pub(crate) fn write_whole(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let folder = path
        .parent()
        .with_context(|| format!("{} has no folder", path.display()))?;
    make_folder(folder)?;
    let partial = path.with_extension("partial");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(&partial)
        .with_context(|| format!("could not write {}", partial.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&partial, path).with_context(|| format!("could not write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use meka_studio::desktop::CloseAction;

    use super::*;

    #[test]
    fn settings_missing_from_the_file_take_their_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(read_settings(&path), DesktopSettings::default());
        std::fs::write(&path, r#"{"close":"quit"}"#).unwrap();
        let settings = read_settings(&path);
        assert_eq!(settings.close, CloseAction::Quit);
        assert!(settings.notifications && settings.check_updates);
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(read_settings(&path), DesktopSettings::default());
    }

    #[test]
    fn a_marker_beside_the_program_makes_a_portable_copy_on_windows_alone() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("meka-studio-desktop.exe");
        assert_eq!(portable_root(&exe), None);
        std::fs::create_dir(dir.path().join(PORTABLE_MARKER)).unwrap();
        assert_eq!(portable_root(&exe), None, "a folder is not the marker");
        std::fs::remove_dir(dir.path().join(PORTABLE_MARKER)).unwrap();
        std::fs::write(dir.path().join(PORTABLE_MARKER), "").unwrap();
        assert_eq!(
            portable_root(&exe),
            cfg!(windows).then(|| dir.path().join("data"))
        );
    }

    #[test]
    fn a_portable_copy_keeps_everything_in_its_folder() {
        let root = PathBuf::from("copy").join("data");
        let paths = Paths::portable(root.clone(), PathBuf::from("meka"));
        let webview = paths.webview.clone().unwrap();
        for path in [
            &paths.meka_config,
            &paths.meka_data,
            &paths.meka_token,
            &paths.studio,
            &paths.settings,
            &paths.memory,
            &paths.logs,
            &webview,
        ] {
            assert!(path.starts_with(&root), "{} is outside", path.display());
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_drive_path_loses_its_verbatim_prefix() {
        assert_eq!(
            plain(PathBuf::from(r"\\?\C:\Program Files\Meka Studio Desktop")),
            PathBuf::from(r"C:\Program Files\Meka Studio Desktop")
        );
        let share = PathBuf::from(r"\\?\UNC\server\share");
        assert_eq!(plain(share.clone()), share);
    }

    #[test]
    fn a_file_is_written_whole_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/memory.json");
        let memory = Memory {
            port: Some(23456),
            told_update: Some("1.2.3".to_owned()),
            window: Some(WindowPlace {
                x: -1200,
                y: 40,
                width: 1280,
                height: 840,
                maximized: true,
            }),
        };
        write_json(&path, &memory).unwrap();
        assert_eq!(read_memory(&path), memory);
        assert!(!path.with_extension("partial").exists());
    }
}

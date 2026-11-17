//! Where the app keeps what it keeps, and the small files it keeps there: its settings, and what
//! it remembers from one start to the next. Meka's config and store are the app's own, apart from
//! any other Meka on the computer.

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

/// The app's folders and files.
#[derive(Debug, Clone)]
pub(crate) struct Paths {
    /// Meka's config folder, its `MEKA_CONFIG_DIR`.
    pub(crate) meka_config: PathBuf,
    /// Meka's data folder, its `MEKA_DATA_DIR`.
    pub(crate) meka_data: PathBuf,
    /// The file holding the token Meka takes from the app.
    pub(crate) meka_token: PathBuf,
    /// The Meka binary the app was built with.
    pub(crate) meka: PathBuf,
    /// Meka Studio's state folder.
    pub(crate) studio: PathBuf,
    /// The app's settings.
    pub(crate) settings: PathBuf,
    /// What the app remembers from one start to the next.
    pub(crate) memory: PathBuf,
    /// The file the running Meka's process id is kept in.
    pub(crate) meka_pid: PathBuf,
    /// The logs' folder.
    pub(crate) logs: PathBuf,
}

impl Paths {
    /// The folders the system gives the app.
    pub(crate) fn of(app: &AppHandle) -> anyhow::Result<Self> {
        let path = app.path();
        let config = path.app_config_dir()?;
        let data = path.app_data_dir()?;
        Ok(Self {
            meka_config: config.join("meka-config"),
            meka_data: data.join("meka-data"),
            meka_token: data.join("meka-token"),
            meka: plain(path.resource_dir()?).join(format!("meka{}", std::env::consts::EXE_SUFFIX)),
            studio: data.join("studio"),
            settings: config.join("settings.json"),
            memory: data.join("memory.json"),
            meka_pid: data.join("meka.pid"),
            logs: path.app_log_dir()?,
        })
    }

    /// Makes the folders that are not there yet, readable by their owner alone.
    pub(crate) fn make(&self) -> anyhow::Result<()> {
        for folder in [&self.meka_config, &self.meka_data, &self.studio, &self.logs] {
            make_folder(folder)?;
        }
        Ok(())
    }
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

    #[cfg(windows)]
    #[test]
    fn a_drive_path_loses_its_verbatim_prefix() {
        assert_eq!(
            plain(PathBuf::from(r"\\?\C:\Program Files\Meka Studio")),
            PathBuf::from(r"C:\Program Files\Meka Studio")
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
        };
        write_json(&path, &memory).unwrap();
        assert_eq!(read_memory(&path), memory);
        assert!(!path.with_extension("partial").exists());
    }
}

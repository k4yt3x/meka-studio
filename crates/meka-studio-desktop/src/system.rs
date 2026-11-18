//! What the app tells Windows of itself: that an installed copy runs, for its installer to see, why
//! it could not start, and, in a portable copy, the name its notifications show under. Windows
//! shows a program's notifications only under a registered name: an installed app's Start menu
//! shortcut registers it, and a portable copy registers it in the user's registry while its
//! notifications are on, marking the entry as its own, so that it removes no entry it did not make.

use std::path::Path;

use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR},
    System::{
        Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPENED_EXISTING_KEY,
            REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteKeyW,
            RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
        },
        Threading::CreateMutexW,
    },
    UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
};

/// The mutex held while an installed copy runs, which its installer waits on before it replaces or
/// removes the app. A portable copy holds none, so the installer leaves one running alone. The
/// installer's script, `windows/meka-studio-desktop.iss`, names it too.
const RUNNING_MUTEX: &str = "bot.meka.studio.running";

/// The value that marks a registered notification name as a portable copy's, holding that copy's
/// data folder.
const MADE_BY: &str = "MekaStudioDesktopPortable";

/// Holds [`RUNNING_MUTEX`] until the app ends, when the system lets it go.
pub(crate) fn hold_running_mutex() {
    let name = wide(RUNNING_MUTEX);
    // SAFETY: `name` is NUL-terminated and outlives the call. The handle is never closed.
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        tracing::warn!(
            "the installer cannot tell that the app runs: {}",
            std::io::Error::last_os_error()
        );
    }
}

/// Registers `app_id`, the name the app's notifications go out under, as `name` with `icon`,
/// marked as made by the portable copy keeping its data in `folder`. An entry made otherwise is
/// left as it is.
pub(crate) fn register_app_id(
    app_id: &str,
    name: &str,
    icon: &Path,
    folder: &Path,
) -> anyhow::Result<()> {
    let path = wide(&app_id_key(app_id));
    let mut key: HKEY = std::ptr::null_mut();
    let mut disposition = 0;
    // SAFETY: the strings are NUL-terminated and outlive the call; the out pointers are valid.
    check(unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            std::ptr::null(),
            &raw mut key,
            &raw mut disposition,
        )
    })?;
    let key = Key(key);
    if disposition == REG_OPENED_EXISTING_KEY && !key.has(MADE_BY)? {
        tracing::info!(
            "leaving the registered notification name {app_id}, which the app did not make"
        );
        return Ok(());
    }
    // Marked first, so an entry left half-written is still the app's to remove.
    key.set(MADE_BY, &folder.display().to_string())?;
    key.set("DisplayName", name)?;
    key.set("IconUri", &icon.display().to_string())
}

/// Removes the registered name `app_id`, if a portable copy made it.
pub(crate) fn unregister_app_id(app_id: &str) -> anyhow::Result<()> {
    let path = wide(&app_id_key(app_id));
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: `path` is NUL-terminated and outlives the call; the out pointer is valid.
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &raw mut key,
        )
    };
    if opened == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check(opened)?;
    let made_here = Key(key).has(MADE_BY)?;
    if !made_here {
        return Ok(());
    }
    // SAFETY: `path` is NUL-terminated and outlives the call.
    check(unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, path.as_ptr()) })
}

/// Where the registered name `app_id` is, under `HKEY_CURRENT_USER`.
fn app_id_key(app_id: &str) -> String {
    format!(r"Software\Classes\AppUserModelId\{app_id}")
}

/// An open registry key, closed when dropped.
struct Key(HKEY);

impl Key {
    /// Whether the key has the value `name`.
    fn has(&self, name: &str) -> anyhow::Result<bool> {
        let name = wide(name);
        // SAFETY: `name` is NUL-terminated and outlives the call; no data is asked for.
        let found = unsafe {
            RegQueryValueExW(
                self.0,
                name.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if found == ERROR_FILE_NOT_FOUND {
            return Ok(false);
        }
        check(found).map(|()| true)
    }

    /// Sets the string value `name` to `value`.
    fn set(&self, name: &str, value: &str) -> anyhow::Result<()> {
        let name = wide(name);
        let value = wide(value);
        let bytes = u32::try_from(value.len() * 2)?;
        // SAFETY: both strings are NUL-terminated and outlive the call, and `bytes` is the size of
        // `value`, its NUL included, as REG_SZ wants.
        check(unsafe {
            RegSetValueExW(
                self.0,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                bytes,
            )
        })
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the key was opened, and is closed once.
        unsafe { RegCloseKey(self.0) };
    }
}

/// Shows `message` in a message box titled `title`, until it is closed.
pub(crate) fn message_box(title: &str, message: &str) {
    let text = wide(message);
    let title = wide(title);
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// `status` as a result.
fn check(status: WIN32_ERROR) -> anyhow::Result<()> {
    if status == ERROR_SUCCESS {
        return Ok(());
    }
    let code = i32::try_from(status)?;
    Err(std::io::Error::from_raw_os_error(code).into())
}

/// `text` as a NUL-terminated UTF-16 string.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

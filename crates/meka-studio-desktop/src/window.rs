//! The windows and the tray icon. A window shows Meka Studio, from its server on loopback, and
//! nothing else: links to other sites open in the browser, downloads go to the Downloads folder,
//! and a page of Meka Studio asked for in a new window opens in another of the app's windows.

use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

use tauri::{
    AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    webview::{DownloadEvent, NewWindowResponse},
};
use tauri_plugin_opener::OpenerExt;
use url::Url;

use crate::shell::{Opens, Shell};

/// The main window's label.
pub(crate) const MAIN: &str = "main";

/// How the main window's title bar is drawn where the page has a part in it, which the page reads
/// from the global `__MEKA_STUDIO_TITLE_BAR__`, set before its scripts run: by the page, with the
/// window's buttons, on Windows; by macOS, its buttons over the page. Elsewhere the system draws
/// it, as window managers differ too much for the page to, and the global is left unset.
#[cfg(windows)]
const TITLE_BAR: Option<&str> = Some("app");
#[cfg(target_os = "macos")]
const TITLE_BAR: Option<&str> = Some("overlay");
#[cfg(not(any(windows, target_os = "macos")))]
const TITLE_BAR: Option<&str> = None;

/// The window commands the page calls where it draws the title bar: dragging the window by its
/// top rows, double-clicking them, and the buttons. The page can call nothing else of the app.
const TITLE_BAR_PERMISSIONS: [&str; 6] = [
    "core:window:allow-start-dragging",
    "core:window:allow-internal-toggle-maximize",
    "core:window:allow-minimize",
    "core:window:allow-toggle-maximize",
    "core:window:allow-close",
    "core:window:allow-is-maximized",
];

/// Whether `url` is Meka Studio at `origin`.
fn is_studio(url: &Url, origin: &Url) -> bool {
    url.origin() == origin.origin()
}

/// Whether `url` is the app's own start page, which the window shows until Meka Studio is up.
fn is_start_page(url: &Url) -> bool {
    // `tauri://localhost` on Linux and macOS, `http://tauri.localhost` on Windows.
    url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost")
}

/// A window builder that keeps the window to Meka Studio at `origin`.
fn builder<'a>(
    app: &'a AppHandle,
    label: &str,
    url: WebviewUrl,
    origin: &Url,
) -> WebviewWindowBuilder<'a, Wry, AppHandle> {
    let navigating = (app.clone(), origin.clone());
    let opening = (app.clone(), origin.clone());
    let downloading = app.clone();
    WebviewWindowBuilder::new(app, label, url)
        .title("Meka Studio")
        // Files takes files dropped on it, which the webview hands over only without the app's
        // own handler.
        .disable_drag_drop_handler()
        .on_navigation(move |url| {
            let (app, origin) = &navigating;
            if is_studio(url, origin) || is_start_page(url) || url.as_str() == "about:blank" {
                return true;
            }
            open_outside(app, url);
            false
        })
        .on_new_window(move |url, _features| {
            let (app, origin) = &opening;
            if is_studio(&url, origin) {
                open_view(app, url, origin.clone());
            } else {
                open_outside(app, &url);
            }
            NewWindowResponse::Deny
        })
        .on_download(move |_webview, event| {
            if let DownloadEvent::Finished { path, success, .. } = event {
                downloaded(&downloading, path.as_deref(), success);
            }
            true
        })
}

/// Opens the main window, at the start page until [`enter`] opens Meka Studio at `origin`. Closing
/// it hides it or quits, as the settings say.
pub(crate) fn open_main(app: &AppHandle, origin: &Url) -> tauri::Result<WebviewWindow> {
    let window = builder(app, MAIN, WebviewUrl::App("index.html".into()), origin)
        .inner_size(1280.0, 840.0)
        .min_inner_size(400.0, 480.0);
    let window = match TITLE_BAR {
        Some(title_bar) => window
            .initialization_script(format!("window.__MEKA_STUDIO_TITLE_BAR__ = '{title_bar}';")),
        None => window,
    };
    #[cfg(target_os = "macos")]
    let window = window
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(tauri::LogicalPosition::new(16.0, 18.0));
    let window = window.build()?;
    if TITLE_BAR.is_some() {
        app.add_capability(
            TITLE_BAR_PERMISSIONS.iter().fold(
                tauri::ipc::CapabilityBuilder::new("title-bar")
                    .remote(format!("{}/*", origin.as_str().trim_end_matches('/')))
                    .window(MAIN),
                |capability, permission| capability.permission(*permission),
            ),
        )?;
    }
    if let Err(error) = fit_to_screen(&window) {
        tracing::warn!("could not fit the window to the screen: {error}");
    }
    let closing = app.clone();
    let hidden = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let shell = closing.state::<Arc<Shell>>();
            match shell.settings().close {
                meka_studio::desktop::CloseAction::Tray => {
                    if let Err(error) = hidden.hide() {
                        tracing::warn!("could not hide the window: {error}");
                    }
                }
                meka_studio::desktop::CloseAction::Quit => crate::quit(&closing),
            }
        }
    });
    Ok(window)
}

/// Shrinks `window` to fit the screen it opens on, and centers it there, if it reaches past the
/// screen's work area: the default size on a small or scaled screen, or a size kept from a larger
/// one.
fn fit_to_screen(window: &WebviewWindow) -> tauri::Result<()> {
    let Some(monitor) = window.current_monitor()? else {
        return Ok(());
    };
    let area = monitor.work_area().size;
    let size = window.outer_size()?;
    if size.width <= area.width && size.height <= area.height {
        return Ok(());
    }
    window.set_size(tauri::PhysicalSize::new(
        size.width.min(area.width / 10 * 9),
        size.height.min(area.height / 10 * 9),
    ))?;
    window.center()
}

/// Opens a page of Meka Studio asked for in a new window, such as a PDF, in a window of its own.
fn open_view(app: &AppHandle, url: Url, origin: Url) {
    static VIEWS: AtomicU32 = AtomicU32::new(0);
    let label = format!("view-{}", VIEWS.fetch_add(1, Ordering::Relaxed));
    let app = app.clone();
    // Made off the webview's own callback, which WebView2 deadlocks in.
    tauri::async_runtime::spawn(async move {
        if let Err(error) = builder(&app, &label, WebviewUrl::External(url), &origin)
            .inner_size(1000.0, 760.0)
            .build()
        {
            tracing::warn!("could not open a window: {error}");
        }
    });
}

/// Opens `url`, of another site, in the browser.
pub(crate) fn open_outside(app: &AppHandle, url: &Url) {
    if !matches!(url.scheme(), "http" | "https" | "mailto") {
        tracing::warn!("not opening {} outside the app", url.scheme());
        return;
    }
    if let Err(error) = app.opener().open_url(url.as_str(), None::<&str>) {
        tracing::warn!("could not open a link in the browser: {error}");
    }
}

/// Tells of a download that finished, or failed.
fn downloaded(app: &AppHandle, path: Option<&std::path::Path>, success: bool) {
    let shell = app.state::<Arc<Shell>>();
    let name = path
        .and_then(std::path::Path::file_name)
        .map(|name| name.to_string_lossy().into_owned());
    if success {
        shell.show(
            "Downloaded",
            name.as_deref()
                .unwrap_or("The download is in your Downloads folder."),
            path.map(|path| Opens::File(path.display().to_string())),
        );
    } else {
        shell.show(
            "Download failed",
            name.as_deref().unwrap_or("A download failed."),
            None,
        );
    }
}

/// Shows the main window, and brings it to the front.
pub(crate) fn show_main(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN) else {
        return;
    };
    let shown = window
        .unminimize()
        .and_then(|()| window.show())
        .and_then(|()| window.set_focus());
    if let Err(error) = shown {
        tracing::warn!("could not show the window: {error}");
    }
}

/// Opens Meka Studio in the main window, at `entry`, which signs the window in.
pub(crate) fn enter(app: &AppHandle, entry: &str) -> anyhow::Result<()> {
    let window = app
        .get_webview_window(MAIN)
        .ok_or_else(|| anyhow::anyhow!("the window is gone"))?;
    // The start page has no buttons of its own, so the system's title bar stays until Meka Studio,
    // which draws the window's, opens: a start that fails can still be closed.
    if TITLE_BAR == Some("app") {
        window.set_decorations(false)?;
    }
    window.navigate(entry.parse()?)?;
    Ok(())
}

/// Shows the Meka session `session` in the main window.
pub(crate) fn open_session(app: &AppHandle, session: &str) {
    show_main(app);
    let Some(window) = app.get_webview_window(MAIN) else {
        return;
    };
    let Ok(id) = serde_json::to_string(session) else {
        return;
    };
    if let Err(error) = window.eval(format!(
        "location.hash = '/sessions/' + encodeURIComponent({id});"
    )) {
        tracing::warn!("could not open the session: {error}");
    }
}

/// Shows `message` on the start page, where the app stopped starting. It goes in the page's
/// address, which the page reads as it loads and when it changes; a start can fail before the
/// window has loaded the page at all, so the window is waited for, up to 10 seconds.
pub(crate) fn show_error(app: &AppHandle, message: &str) {
    show_main(app);
    let app = app.clone();
    let message: String = url::form_urlencoded::byte_serialize(message.as_bytes()).collect();
    tauri::async_runtime::spawn(async move {
        for _ in 0..100 {
            if let Some(window) = app.get_webview_window(MAIN)
                && let Ok(mut url) = window.url()
                && is_start_page(&url)
            {
                url.set_fragment(Some(&format!("error={message}")));
                if let Err(error) = window.navigate(url) {
                    tracing::warn!("could not show why the app stopped starting: {error}");
                }
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        tracing::warn!("could not show why the app stopped starting: the start page did not load");
    });
}

/// The tray icon: a click shows the window, and its menu shows it or quits.
pub(crate) fn tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Meka Studio", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Meka Studio", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Meka Studio")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "quit" => crate::quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_stays_on_meka_studio_and_its_start_page() {
        let origin: Url = "http://127.0.0.1:23456".parse().unwrap();
        let at = |url: &str| url.parse::<Url>().unwrap();
        assert!(is_studio(
            &at("http://127.0.0.1:23456/#/sessions/x"),
            &origin
        ));
        assert!(!is_studio(&at("http://127.0.0.1:23457/"), &origin));
        assert!(!is_studio(&at("http://localhost:23456/"), &origin));
        assert!(!is_studio(&at("https://127.0.0.1:23456/"), &origin));
        assert!(is_start_page(&at("tauri://localhost/index.html")));
        assert!(is_start_page(&at("http://tauri.localhost/index.html")));
        assert!(!is_start_page(&at("https://example.com/")));
    }
}

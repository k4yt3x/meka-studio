//! The windows and the tray icon. A window shows Meka Studio, from its server on loopback, and
//! nothing else: links to other sites open in the browser, downloads go to the Downloads folder,
//! and a page of Meka Studio asked for in a new window opens in another of the app's windows.

use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

use meka_studio::desktop::{CloseAction, DesktopSettings, Host};
use tauri::{
    AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    webview::{DownloadEvent, NewWindowResponse},
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogResult};
use tauri_plugin_opener::OpenerExt;
use url::Url;

use crate::{
    files::WindowPlace,
    shell::{Opens, Shell},
};

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
    let builder = WebviewWindowBuilder::new(app, label, url);
    let builder = match app
        .try_state::<Arc<Shell>>()
        .and_then(|shell| shell.paths.webview.clone())
    {
        Some(folder) => builder.data_directory(folder),
        None => builder,
    };
    builder
        .title(&app.package_info().name)
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

/// Opens the main window where it was before, at the start page until [`enter`] opens Meka Studio
/// at `origin`. Closing it hides it or quits, as the settings say.
pub(crate) fn open_main(app: &AppHandle, origin: &Url) -> tauri::Result<WebviewWindow> {
    let window = builder(app, MAIN, WebviewUrl::App("index.html".into()), origin)
        .inner_size(1280.0, 840.0)
        .min_inner_size(400.0, 480.0)
        // Shown once it is in its place.
        .visible(false);
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
    crate::drops::tell_paths(&window, origin);
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
    let shell = app.state::<Arc<Shell>>();
    let kept = put_back(&window, shell.memory().window).unwrap_or_else(|error| {
        tracing::warn!("could not put the window where it was: {error}");
        None
    });
    if let Err(error) = fit_to_screen(&window, kept.is_none()) {
        tracing::warn!("could not fit the window to the screen: {error}");
    }
    // After fitting it, as sizing a window restores it.
    if kept == Some(true) {
        window.maximize()?;
    }
    window.show()?;
    shell.note_place(&window, false);
    let app = app.clone();
    let main = window.clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::Moved(_) => app.state::<Arc<Shell>>().note_place(&main, true),
        tauri::WindowEvent::Resized(_) => app.state::<Arc<Shell>>().note_place(&main, false),
        tauri::WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            match app.state::<Arc<Shell>>().settings().close {
                CloseAction::Ask => ask_on_close(&app, &main),
                CloseAction::Tray => hide_main(&app, &main),
                CloseAction::Quit => crate::quit(&app),
            }
        }
        _ => {}
    });
    Ok(window)
}

/// Hides the main window `main` to the tray, keeping its place.
fn hide_main(app: &AppHandle, main: &WebviewWindow) {
    app.state::<Arc<Shell>>().keep_place(main);
    if let Err(error) = main.hide() {
        tracing::warn!("could not hide the window: {error}");
    }
}

/// Asks whether closing the main window `main` keeps the app running in the tray or quits, as the
/// settings leave it to the window's first close, keeps the answer in them, and does it. Dismissed,
/// the question leaves the window open.
fn ask_on_close(app: &AppHandle, main: &WebviewWindow) {
    const KEEP: &str = "Keep running";
    const QUIT: &str = "Quit";
    let answering = (app.clone(), main.clone());
    app.dialog()
        .message(
            "Keep Meka running in the tray when the window closes? Quitting stops Meka and its \
             turns. Settings → General changes this.",
        )
        .title(app.package_info().name.clone())
        .parent(main)
        .buttons(MessageDialogButtons::YesNoCancelCustom(
            KEEP.to_owned(),
            QUIT.to_owned(),
            "Cancel".to_owned(),
        ))
        .show_with_result(move |answer| {
            let (app, main) = answering;
            let close = match answer {
                MessageDialogResult::Custom(label) if label == KEEP => CloseAction::Tray,
                MessageDialogResult::Custom(label) if label == QUIT => CloseAction::Quit,
                _ => return,
            };
            let shell = app.state::<Arc<Shell>>();
            let settings = DesktopSettings {
                close,
                ..shell.settings()
            };
            if let Err(error) = shell.save_settings(settings) {
                tracing::warn!("could not keep what closing the window does: {error:#}");
            }
            match close {
                CloseAction::Quit => crate::quit(&app),
                _ => hide_main(&app, &main),
            }
        });
}

/// Puts `window` back in its place `kept`, if some of that is on a screen still, and returns
/// whether it was maximized there; otherwise, `None`, it opens where the system puts a new window.
fn put_back(window: &WebviewWindow, kept: Option<WindowPlace>) -> tauri::Result<Option<bool>> {
    let Some(kept) = kept else {
        return Ok(None);
    };
    let areas: Vec<_> = window
        .available_monitors()?
        .iter()
        .map(|monitor| *monitor.work_area())
        .collect();
    if !on_screen(&kept, &areas) {
        return Ok(None);
    }
    window.set_position(tauri::PhysicalPosition::new(kept.x, kept.y))?;
    set_outer_size(window, tauri::PhysicalSize::new(kept.width, kept.height))?;
    Ok(Some(kept.maximized))
}

/// Sizes `window` by its outer edges, as its place is kept: the system's title bar comes and goes
/// between the start page and Meka Studio, and a window is sized by what is inside its frame.
fn set_outer_size(window: &WebviewWindow, size: tauri::PhysicalSize<u32>) -> tauri::Result<()> {
    let outer = window.outer_size()?;
    let inner = window.inner_size()?;
    window.set_size(tauri::PhysicalSize::new(
        size.width
            .saturating_sub(outer.width.saturating_sub(inner.width)),
        size.height
            .saturating_sub(outer.height.saturating_sub(inner.height)),
    ))
}

/// Whether enough of a window at `place` lies on one of the screens' work `areas` to take hold of
/// and move.
fn on_screen(place: &WindowPlace, areas: &[tauri::PhysicalRect<i32, u32>]) -> bool {
    /// How much of the window must be on one screen, across and down.
    const SHOWN: (i64, i64) = (100, 50);
    let (left, top) = (i64::from(place.x), i64::from(place.y));
    let (right, bottom) = (left + i64::from(place.width), top + i64::from(place.height));
    areas.iter().any(|area| {
        let (area_left, area_top) = (i64::from(area.position.x), i64::from(area.position.y));
        let across = right.min(area_left + i64::from(area.size.width)) - left.max(area_left);
        let down = bottom.min(area_top + i64::from(area.size.height)) - top.max(area_top);
        across >= SHOWN.0 && down >= SHOWN.1
    })
}

/// Where `window` is now, unless it is maximized or minimized, which leaves its place as it was.
pub(crate) fn normal_place(window: &WebviewWindow) -> tauri::Result<Option<WindowPlace>> {
    if window.is_maximized()? || window.is_minimized()? {
        return Ok(None);
    }
    let position = window.outer_position()?;
    let size = window.outer_size()?;
    Ok((size.width > 0 && size.height > 0).then_some(WindowPlace {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
        maximized: false,
    }))
}

/// Centers `window` on the screen it opens on if it reaches past the screen's work area, shrinking
/// it first where it is larger: the default size on a small or scaled screen, or a size kept from
/// a larger one. A window the system `placed` is centered where it reaches past the area at all, as
/// Windows may put a new window's foot under the taskbar; a place kept is the person's, and may
/// hang past an edge.
fn fit_to_screen(window: &WebviewWindow, placed: bool) -> tauri::Result<()> {
    let Some(monitor) = window.current_monitor()? else {
        return Ok(());
    };
    let area = *monitor.work_area();
    let size = window.outer_size()?;
    if size.width > area.size.width || size.height > area.size.height {
        set_outer_size(
            window,
            tauri::PhysicalSize::new(
                size.width.min(area.size.width / 10 * 9),
                size.height.min(area.size.height / 10 * 9),
            ),
        )?;
    } else if !placed || within(window.outer_position()?, size, &area) {
        return Ok(());
    }
    window.center()
}

/// Whether a window at `position` of `size` lies wholly in the work `area`.
fn within(
    position: tauri::PhysicalPosition<i32>,
    size: tauri::PhysicalSize<u32>,
    area: &tauri::PhysicalRect<i32, u32>,
) -> bool {
    let (left, top) = (i64::from(position.x), i64::from(position.y));
    let (area_left, area_top) = (i64::from(area.position.x), i64::from(area.position.y));
    left >= area_left
        && top >= area_top
        && left + i64::from(size.width) <= area_left + i64::from(area.size.width)
        && top + i64::from(size.height) <= area_top + i64::from(area.size.height)
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

/// Says why the app could not start in a message box, and ends. A window opened only to say so
/// would keep its webview's data in the user's profile, which a portable copy must not touch.
#[cfg(windows)]
pub(crate) fn fail_to_start(app: &AppHandle, message: &str) {
    tracing::error!("{message}");
    crate::system::message_box(&app.package_info().name, message);
    std::process::exit(1);
}

/// Says why the app could not start on the start page: in the main window if it is open, or else
/// in a window whose closing ends the app.
#[cfg(not(windows))]
pub(crate) fn fail_to_start(app: &AppHandle, message: &str) {
    tracing::error!("{message}");
    eprintln!("{message}");
    if app.get_webview_window(MAIN).is_some() {
        show_error(app, message);
        return;
    }
    let window = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title(&app.package_info().name)
        .inner_size(640.0, 400.0);
    // Where the app's windows keep their data, rather than in a folder Tauri would make for it.
    #[cfg(not(target_os = "macos"))]
    let window = match crate::files::installed_webview(app) {
        Ok(folder) => window.data_directory(folder),
        Err(error) => {
            eprintln!("could not find the app's folders: {error}");
            window
        }
    };
    match window.build() {
        Ok(_) => show_error(app, message),
        Err(error) => {
            eprintln!("could not open a window to say so: {error}");
            std::process::exit(1);
        }
    }
}

/// The tray icon: a click shows the window, and its menu shows it or quits.
pub(crate) fn tray(app: &AppHandle) -> tauri::Result<()> {
    let name = &app.package_info().name;
    let show = MenuItem::with_id(app, "show", format!("Show {name}"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", format!("Quit {name}"), true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip(name)
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

    #[test]
    fn a_window_goes_back_only_to_a_place_it_can_be_moved_from() {
        let area = |x, y, width, height| tauri::PhysicalRect {
            position: tauri::PhysicalPosition::new(x, y),
            size: tauri::PhysicalSize::new(width, height),
        };
        let place = |x, y| WindowPlace {
            x,
            y,
            width: 1280,
            height: 840,
            maximized: false,
        };
        let screens = [area(0, 0, 1920, 1040), area(-1600, 200, 1600, 860)];
        assert!(on_screen(&place(100, 100), &screens));
        assert!(
            on_screen(&place(-1500, 300), &screens),
            "on the left screen"
        );
        assert!(on_screen(&place(1820, 990), &screens), "100 by 50 shown");
        assert!(!on_screen(&place(1821, 100), &screens), "99 across shown");
        assert!(!on_screen(&place(100, 991), &screens), "49 down shown");
        assert!(
            !on_screen(&place(-3000, 300), &screens),
            "a screen since gone"
        );
        assert!(!on_screen(&place(100, 100), &[]));
    }

    #[test]
    fn a_window_reaching_under_the_taskbar_is_not_within_the_work_area() {
        let area = tauri::PhysicalRect {
            position: tauri::PhysicalPosition::new(-1920, 0),
            size: tauri::PhysicalSize::new(1920, 1032),
        };
        let size = tauri::PhysicalSize::new(1280, 840);
        let at = |x, y| within(tauri::PhysicalPosition::new(x, y), size, &area);
        assert!(at(-1920, 0), "in the corner");
        assert!(at(-1280, 192), "against the far corner");
        assert!(!at(-1920, 193), "its foot under the taskbar");
        assert!(!at(-1279, 100), "past the right edge");
        assert!(!at(-1921, 100), "past the left edge");
        assert!(!at(-1920, -1), "above the top");
    }
}

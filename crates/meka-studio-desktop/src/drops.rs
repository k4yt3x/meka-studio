//! The paths of files and folders dropped on the main window, which its webview keeps from the
//! page: the page has the files' contents, but not where they are on this computer.
//!
//! On Linux the app sees each drop's addresses before WebKitGTK hands the drop to the page, and
//! sets them as the global `__MEKA_STUDIO_DROPPED__` first. On Windows the page asks: it posts the
//! dropped files with WebView2's `postMessageWithAdditionalObjects`, and the app answers with their
//! paths. The question is a string, since WebView2 stops at the first handler of a message that
//! fails, and Tauri's, which comes first, fails on any other; Tauri then logs the question to the
//! page's console as a call it cannot read. macOS offers neither, so there the page has none.

use tauri::WebviewWindow;
use url::Url;

/// Tells the page in `window`, at `origin`, where the files and folders dropped on it are.
#[cfg(target_os = "linux")]
pub(crate) fn tell_paths(window: &WebviewWindow, _origin: &Url) {
    use gtk::prelude::WidgetExt;
    use webkit2gtk::WebViewExt;
    let installed = window.with_webview(|webview| {
        // Before WebKit's own handler, which hands the drop to the page.
        webview
            .inner()
            .connect_drag_data_received(|view, _, _, _, data, _, _| {
                let paths: Vec<String> = data
                    .uris()
                    .iter()
                    .filter_map(|uri| Url::parse(uri).ok()?.to_file_path().ok())
                    .filter_map(|path| path.into_os_string().into_string().ok())
                    .collect();
                let script = format!(
                    "window.__MEKA_STUDIO_DROPPED__ = {};",
                    serde_json::Value::from(paths)
                );
                view.evaluate_javascript(
                    &script,
                    None,
                    None,
                    None::<&webkit2gtk::gio::Cancellable>,
                    |result| {
                        if let Err(error) = result {
                            tracing::warn!("could not tell the page where a drop is from: {error}");
                        }
                    },
                );
            });
    });
    if let Err(error) = installed {
        tracing::warn!("could not watch the window for dropped files: {error}");
    }
}

/// Tells the page in `window`, at `origin`, where the files and folders dropped on it are.
#[cfg(windows)]
pub(crate) fn tell_paths(window: &WebviewWindow, origin: &Url) {
    use webview2_com::WebMessageReceivedEventHandler;
    let origin = origin.origin();
    let installed = window.with_webview(move |webview| {
        // SAFETY: COM calls on the window's own webview, on the thread it runs on.
        let added = unsafe {
            webview.controller().CoreWebView2().and_then(|core| {
                let mut token = 0;
                core.add_WebMessageReceived(
                    &WebMessageReceivedEventHandler::create(Box::new(move |sender, asked| {
                        if let (Some(sender), Some(asked)) = (sender, asked)
                            && let Err(error) = webview2::answer(&sender, &asked, &origin)
                        {
                            tracing::warn!("could not tell the page where a drop is from: {error}");
                        }
                        Ok(())
                    })),
                    &mut token,
                )
            })
        };
        if let Err(error) = added {
            tracing::warn!("could not watch the window for dropped files: {error}");
        }
    });
    if let Err(error) = installed {
        tracing::warn!("could not watch the window for dropped files: {error}");
    }
}

/// Tells the page in `window` nothing: WKWebView hands a drop to the page without its paths, and
/// lets no one else see it.
#[cfg(target_os = "macos")]
pub(crate) const fn tell_paths(_window: &WebviewWindow, _origin: &Url) {}

#[cfg(windows)]
mod webview2 {
    use serde::Deserialize;
    use url::{Origin, Url};
    use webview2_com::{
        Microsoft::Web::WebView2::Win32::{
            ICoreWebView2, ICoreWebView2File, ICoreWebView2WebMessageReceivedEventArgs,
            ICoreWebView2WebMessageReceivedEventArgs2,
        },
        take_pwstr,
    };
    use windows_core::{HSTRING, Interface, PWSTR};

    /// What the page asks, as a string, with the dropped files beside it: `type` is
    /// `dropped-paths`.
    #[derive(Deserialize)]
    struct Asked {
        r#type: String,
        id: u64,
    }

    /// Answers a page at `origin` asking where the files it posted are, with their paths in the
    /// order it posted them. Any other message is not the app's.
    pub(super) fn answer(
        sender: &ICoreWebView2,
        asked: &ICoreWebView2WebMessageReceivedEventArgs,
        origin: &Origin,
    ) -> windows_core::Result<()> {
        // SAFETY: reads of the message WebView2 hands this handler, and a message back to the
        // webview that sent it.
        unsafe {
            let mut source = PWSTR::null();
            asked.Source(&mut source)?;
            if Url::parse(&take_pwstr(source))
                .map(|url| url.origin())
                .as_ref()
                != Ok(origin)
            {
                return Ok(());
            }
            let mut message = PWSTR::null();
            if asked.TryGetWebMessageAsString(&mut message).is_err() {
                return Ok(());
            }
            let Ok(Asked { r#type, id }) = serde_json::from_str(&take_pwstr(message)) else {
                return Ok(());
            };
            if r#type != "dropped-paths" {
                return Ok(());
            }
            let objects = asked
                .cast::<ICoreWebView2WebMessageReceivedEventArgs2>()?
                .AdditionalObjects()?;
            let mut count = 0;
            objects.Count(&mut count)?;
            let mut paths = Vec::new();
            for index in 0..count {
                let Ok(file) = objects.GetValueAtIndex(index)?.cast::<ICoreWebView2File>() else {
                    continue;
                };
                let mut path = PWSTR::null();
                file.Path(&mut path)?;
                paths.push(take_pwstr(path));
            }
            let answer = serde_json::json!({ "type": "dropped-paths", "id": id, "paths": paths });
            sender.PostWebMessageAsJson(&HSTRING::from(answer.to_string()))
        }
    }
}

//! The hidden web view the Player runs in. GPUI owns the NSApplication run loop, and wry needs
//! nothing more than that on macOS: it only wants an NSView to attach its WKWebView to, and
//! WebKit delivers custom protocol requests and script messages on the main thread through the
//! same run loop. So the host is a plain NSWindow that is never ordered on screen.

use std::borrow::Cow;
use std::ptr::NonNull;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Global};
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{
    AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle,
};
use wry::http::{Response, StatusCode};
use wry::{WebView, WebViewBuilder};

const SCHEME: &str = "slopify";
const URL: &str = "slopify://player/";
const PAGE: &str = include_str!("player.html");

pub struct PlayerHost {
    // Declared first so it drops before the window it lives in.
    webview: WebView,
    _window: HostWindow,
}

impl Global for PlayerHost {}

/// Loads the Player page in a hidden web view. Every string the page passes to
/// `window.ipc.postMessage` reaches `on_message` on the main thread with the app in hand.
pub fn init(cx: &mut App, on_message: impl Fn(String, &mut App) + 'static) -> wry::Result<()> {
    let mtm = MainThreadMarker::new().expect("the player host is built on the main thread");
    let window = HostWindow::new(mtm);

    // wry calls the IPC handler outside any GPUI update, so hand messages to a foreground task
    // rather than borrowing the app from inside WebKit's callback.
    let (tx, mut messages) = mpsc::unbounded::<String>();
    let webview = WebViewBuilder::new()
        .with_url(URL)
        .with_autoplay(true)
        .with_custom_protocol(SCHEME.into(), |_id, request| match request.uri().path() {
            "/" => html(PAGE),
            _ => not_found(),
        })
        .with_ipc_handler(move |request| {
            let _ = tx.unbounded_send(request.into_body());
        })
        .build(&window)?;

    cx.spawn(async move |cx| {
        while let Some(message) = messages.next().await {
            cx.update(|cx| on_message(message, cx));
        }
    })
    .detach();

    cx.set_global(PlayerHost {
        webview,
        _window: window,
    });
    Ok(())
}

/// Runs `js` in the Player page.
#[allow(dead_code, reason = "the Player (#27) drives the SDK through this")]
pub fn eval(js: &str, cx: &App) -> wry::Result<()> {
    cx.global::<PlayerHost>().webview.evaluate_script(js)
}

fn html(body: &'static str) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .header("Content-Type", "text/html; charset=utf-8")
        .body(Cow::Borrowed(body.as_bytes()))
        .expect("static response")
}

fn not_found() -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Cow::Borrowed(&[][..]))
        .expect("static response")
}

struct HostWindow(Retained<NSWindow>);

impl HostWindow {
    fn new(mtm: MainThreadMarker) -> Self {
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(480.0, 360.0));
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                frame,
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        Self(window)
    }
}

impl HasWindowHandle for HostWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let view = self.0.contentView().ok_or(HandleError::Unavailable)?;
        let handle = AppKitWindowHandle::new(NonNull::from(&*view).cast());
        // The window retains its content view for as long as `self` lives.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::AppKit(handle)) })
    }
}

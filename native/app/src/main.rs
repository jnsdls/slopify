mod dropdown;
mod player_host;
mod status_item;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::App;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

use crate::status_item::StatusItem;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    log::info!("slopify starting");

    gpui_platform::application().run(|cx: &mut App| {
        let mtm = MainThreadMarker::new().expect("GPUI runs its callback on the main thread");
        // GPUI sets the Regular policy in applicationDidFinishLaunching, which also overrides
        // LSUIElement in a bundle, so this has to run after it either way.
        NSApplication::sharedApplication(mtm)
            .setActivationPolicy(NSApplicationActivationPolicy::Accessory);

        dropdown::init(cx);

        let (clicks_tx, mut clicks) = mpsc::unbounded();
        cx.set_global(StatusItem::new(mtm, move || {
            let _ = clicks_tx.unbounded_send(());
        }));
        cx.spawn(async move |cx| {
            while clicks.next().await.is_some() {
                cx.update(|cx| {
                    if let Some(anchor) = cx.global::<StatusItem>().anchor() {
                        dropdown::toggle(anchor, cx);
                    }
                });
            }
        })
        .detach();

        if let Err(err) = player_host::init(cx, |message, _cx| {
            log::info!("player: {message}");
        }) {
            log::error!("player host failed to start: {err}");
        }
    });
}

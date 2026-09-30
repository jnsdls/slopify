mod app_model;
mod dropdown;
mod login_item;
mod player;
mod player_host;
mod status_item;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::App;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use slopify_state::StateFile;

use crate::app_model::AppModel;
use crate::status_item::StatusItem;

// Public: one per person building the app, read at build time like the Electron app did.
const CLIENT_ID: Option<&str> = option_env!("SLOPIFY_SPOTIFY_CLIENT_ID");

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    log::info!("slopify starting");
    let Some(client_id) = CLIENT_ID.filter(|id| !id.is_empty()) else {
        log::error!(
            "slopify was built without a Spotify client id; rebuild with SLOPIFY_SPOTIFY_CLIENT_ID set"
        );
        std::process::exit(1);
    };
    let Some(state_path) = slopify_state::default_path() else {
        log::error!("no home directory to keep state.json in");
        std::process::exit(1);
    };
    let state_file = StateFile::open(state_path);
    login_item::register_on_first_run(state_file.first_run());

    gpui_platform::application().run(|cx: &mut App| {
        let mtm = MainThreadMarker::new().expect("GPUI runs its callback on the main thread");
        // GPUI sets the Regular policy in applicationDidFinishLaunching, which also overrides
        // LSUIElement in a bundle, so this has to run after it either way.
        NSApplication::sharedApplication(mtm)
            .setActivationPolicy(NSApplicationActivationPolicy::Accessory);

        let model = AppModel::init(client_id, state_file, cx);
        dropdown::init(model.clone(), cx);

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

        if let Err(err) = player_host::init(cx, move |message, cx| {
            model.update(cx, |model, cx| model.on_page_message(&message, cx));
        }) {
            log::error!("player host failed to start: {err}");
        }
    });
}

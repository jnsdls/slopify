//! Open at login, set once on first run. No toggle in the app; System Settings switches it off.

use objc2_foundation::NSBundle;
use objc2_service_management::{SMAppService, SMAppServiceStatus};

pub fn register_on_first_run(first_run: bool) {
    if !first_run {
        return;
    }
    // SMAppService registers the enclosing .app; a bare `cargo run` binary has none.
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        log::info!("login item: not running from a bundle, skipping");
        return;
    }
    let service = unsafe { SMAppService::mainAppService() };
    if unsafe { service.status() } == SMAppServiceStatus::Enabled {
        return;
    }
    match unsafe { service.registerAndReturnError() } {
        Ok(()) => log::info!("login item: registered"),
        Err(err) => log::warn!("login item: register failed: {err}"),
    }
}

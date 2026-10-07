//! The app's one model: auth, the Player, the Sources and the transient message, wired to the
//! token store, the Web API, the state file and the Player page.
//!
//! # For the Dropdown (#28)
//!
//! `main` hands the entity to the Dropdown; `observe` it. It notifies on every change, including
//! the 500 ms position tick.
//!
//! - State: [`AppModel::auth`], [`AppModel::player`] (track, paused, position, duration, volume,
//!   Source, "Playing on" device, message), [`AppModel::fatal`], [`AppModel::sources`],
//!   [`AppModel::status`], [`AppModel::sign_in_view`], [`AppModel::display_name`].
//! - Actions: [`AppModel::sign_in`], [`AppModel::toggle_play`], [`AppModel::next`],
//!   [`AppModel::set_volume`], [`AppModel::start_source`], [`AppModel::refresh_sources`] (call
//!   when the Picker opens), [`AppModel::resolve_pasted_link`] (then `start_source` on success),
//!   [`AppModel::transfer_here`], [`AppModel::open_external`], [`AppModel::quit`].
//! - Pure view rules (`status_of`, `sign_in_line`, `paste_error_line`, `picker_rows`,
//!   `source_name`, `same_source`) are in `player::view`, times and links in `player::format`.
//!
//! # For Now Playing (#29)
//!
//! Subscribe to [`AppEvent::PlaybackChanged`]. It fires from one place, [`AppModel::input`],
//! whenever the track, paused flag, duration, position anchor or "Playing on" state changes; not
//! on the interpolating tick. [`AppModel::play`], [`AppModel::pause`], [`AppModel::next`] and
//! [`AppModel::previous`] are the media key actions.

use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::{App, AppContext, Context, Entity, EventEmitter, Task};
use slopify_auth::{AuthState, SecurityKeychain, TokenStore, UreqHttp};
use slopify_spotify::{
    BridgeError, BridgeErrorCode, Error as ApiError, Source, SourceCatalog, SpotifyApi,
    StartSource, TokenFns, UreqClient, get_playing_elsewhere, resolve_pasted_link, start_source,
    transfer_here,
};
use slopify_state::StateFile;

use crate::player::page::{MediaAction, PageMessage, SdkCommand, answer_token};
use crate::player::view::{self, SignInView, Status};
use crate::player::{CommandError, Effect, Player, PlayerState};
use crate::player_host;

type Catalog = SourceCatalog<Arc<SpotifyApi>>;

/// The blocking clients. Shared with the threads that call them.
struct Services {
    tokens: TokenStore,
    api: Arc<SpotifyApi>,
    catalog: Catalog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEvent {
    PlaybackChanged,
}

pub struct AppModel {
    services: Arc<Services>,
    state_file: StateFile,
    auth: AuthState,
    player: Player,
    sources: Arc<Vec<Source>>,
    device_id: Option<String>,
    sdk_loaded: bool,
    started: bool,
    timer: Option<(Instant, Task<()>)>,
    _auth_updates: slopify_auth::Subscription,
    _quit: gpui::Subscription,
}

impl EventEmitter<AppEvent> for AppModel {}

impl AppModel {
    /// Builds the model and starts auth in the background.
    pub fn init(client_id: &str, state_file: StateFile, cx: &mut App) -> Entity<AppModel> {
        cx.new(|cx| Self::new(client_id, state_file, cx))
    }

    fn new(client_id: &str, state_file: StateFile, cx: &mut Context<Self>) -> Self {
        let tokens = TokenStore::new(client_id, SecurityKeychain::new(), UreqHttp::new());
        let api = Arc::new(SpotifyApi::new(
            TokenFns {
                get: {
                    let tokens = tokens.clone();
                    move || tokens.access_token().map_err(Into::into)
                },
                refresh: {
                    let tokens = tokens.clone();
                    move || tokens.force_refresh().map_err(Into::into)
                },
            },
            UreqClient::new(),
        ));
        let catalog = SourceCatalog::new(api.clone());

        // The store publishes from whatever thread changed it; hop to the main thread.
        let (tx, mut updates) = mpsc::unbounded();
        let auth_updates = tokens.on_state(move |state| {
            let _ = tx.unbounded_send(state.clone());
        });
        cx.spawn(async move |this, cx| {
            while let Some(state) = updates.next().await {
                if this.update(cx, |m, cx| m.on_auth(state, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let start = tokens.clone();
        cx.spawn(async move |_, _| {
            if let Err(err) = blocking(move || start.start()).await {
                log::error!("auth start failed: {err}");
            }
        })
        .detach();

        let quit = cx.on_app_quit(|this, cx| {
            this.input(|p, now| p.on_quit(now), cx);
            this.state_file.flush();
            async {}
        });

        let player = Player::new(state_file.volume(), Instant::now());
        Self {
            auth: tokens.state(),
            services: Arc::new(Services {
                tokens,
                api,
                catalog,
            }),
            state_file,
            player,
            sources: Arc::default(),
            device_id: None,
            sdk_loaded: false,
            started: false,
            timer: None,
            _auth_updates: auth_updates,
            _quit: quit,
        }
    }

    // State

    #[allow(dead_code, reason = "the Dropdown reads auth through sign_in_view")]
    pub fn auth(&self) -> &AuthState {
        &self.auth
    }

    pub fn player(&self) -> &PlayerState {
        self.player.state()
    }

    #[allow(dead_code, reason = "the Dropdown reads it through sign_in_view")]
    pub fn fatal(&self) -> Option<&str> {
        self.player.fatal()
    }

    /// Liked Songs first, then playlists. Empty until the first fetch lands.
    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    pub fn status(&self) -> Status {
        view::status_of(self.player.state())
    }

    /// `None` means the player layout.
    pub fn sign_in_view(&self) -> Option<SignInView> {
        view::sign_in_view(&self.auth, self.player.fatal())
    }

    pub fn display_name(&self) -> Option<&str> {
        match &self.auth {
            AuthState::SignedIn { display_name, .. } => Some(display_name),
            _ => None,
        }
    }

    // Actions

    /// Opens the browser; the auth state goes to signing-in and then signed-in or signed-out.
    pub fn sign_in(&mut self, _cx: &mut Context<Self>) {
        let tokens = self.services.tokens.clone();
        std::thread::spawn(move || tokens.sign_in());
    }

    pub fn toggle_play(&mut self, cx: &mut Context<Self>) {
        self.input(|p, _| p.toggle_play(), cx);
    }

    pub fn play(&mut self, cx: &mut Context<Self>) {
        self.input(|p, _| p.play(), cx);
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        self.input(|p, _| p.pause(), cx);
    }

    pub fn next(&mut self, cx: &mut Context<Self>) {
        self.input(|p, _| p.next(), cx);
    }

    pub fn previous(&mut self, cx: &mut Context<Self>) {
        self.input(|p, _| p.previous(), cx);
    }

    /// 0..=1. Goes to the SDK at once and to the state file 300 ms after the last change.
    pub fn set_volume(&mut self, volume: f64, cx: &mut Context<Self>) {
        self.input(|p, now| p.set_volume(volume, now), cx);
    }

    /// Plays `source` from its start. Failures show as the transient message.
    pub fn start_source(&mut self, source: Source, cx: &mut Context<Self>) {
        self.input(|p, _| p.start_source(source), cx);
    }

    /// Takes the session back from the device in "Playing on".
    #[allow(dead_code, reason = "the Dropdown reaches it through toggle_play")]
    pub fn transfer_here(&mut self, cx: &mut Context<Self>) {
        self.apply(vec![Effect::TransferHere], cx);
    }

    /// Fetches the Source list again. Call when the Picker opens; `sources` keeps the old list
    /// until the new one lands.
    pub fn refresh_sources(&mut self, cx: &mut Context<Self>) {
        let services = self.services.clone();
        cx.spawn(async move |this, cx| {
            let result = blocking(move || services.catalog.list()).await;
            let _ = this.update(cx, |m, cx| match result {
                Ok(sources) => {
                    m.sources = sources.clone();
                    m.player.set_known_sources(sources, m.user_id());
                    cx.notify();
                }
                Err(err) => log::error!("listing sources failed: {err}"),
            });
        })
        .detach();
    }

    /// Turns a pasted link into a Pasted Playlist. The error is the code for
    /// `view::paste_error_line`; `None` means something other than a bad or hidden link.
    pub fn resolve_pasted_link(
        &self,
        text: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<Source, Option<BridgeErrorCode>>> {
        let services = self.services.clone();
        cx.spawn(async move |_, _| {
            blocking(move || resolve_pasted_link(&*services.api, &text))
                .await
                .map_err(|err| match err {
                    ApiError::Bridge(e) => Some(e.code),
                    other => {
                        log::error!("resolving a pasted link failed: {other}");
                        None
                    }
                })
        })
    }

    /// Opens an http(s) link in the default browser.
    pub fn open_external(&self, url: &str) {
        let lower = url.to_ascii_lowercase();
        if !(lower.starts_with("https://") || lower.starts_with("http://")) {
            return;
        }
        if let Err(err) = std::process::Command::new("/usr/bin/open").arg(url).spawn() {
            log::error!("open failed: {err}");
        }
    }

    /// Saves the Resume Point and volume, then quits.
    pub fn quit(&mut self, cx: &mut Context<Self>) {
        cx.quit();
    }

    // Inputs

    /// Everything the Player page posts lands here.
    pub fn on_page_message(&mut self, raw: &str, cx: &mut Context<Self>) {
        let message = match PageMessage::parse(raw) {
            Ok(m) => m,
            Err(err) => {
                log::warn!("player page sent something odd: {err}");
                return;
            }
        };
        match message {
            PageMessage::SdkLoaded => {
                log::info!("player: SDK loaded");
                self.sdk_loaded = true;
                self.maybe_start(cx);
            }
            PageMessage::Ready { device_id } => {
                log::info!("player: ready");
                self.input(|p, _| p.on_ready(device_id), cx);
            }
            PageMessage::NotReady => {
                log::warn!("player: not ready");
                self.input(|p, now| p.on_not_ready(now), cx);
            }
            PageMessage::State { state } => {
                match &state {
                    Some(s) => log::debug!(
                        "player: state {} paused={} position={}",
                        s.track_window.current_track.uri,
                        s.paused,
                        s.position
                    ),
                    None => log::debug!("player: state null"),
                }
                self.input(|p, now| p.on_state(state, now), cx)
            }
            PageMessage::Error { event, message } => {
                log::error!("player: {event} {message}");
                match event.as_str() {
                    "initialization_error" | "authentication_error" => {
                        self.input(|p, _| p.on_auth_error(message), cx)
                    }
                    "account_error" => self.input(|p, _| p.on_account_error(), cx),
                    "playback_error" => self.input(|p, now| p.on_playback_error(message, now), cx),
                    _ => {}
                }
            }
            PageMessage::Token { id } => {
                let services = self.services.clone();
                cx.spawn(async move |_, cx| {
                    let token = blocking(move || services.tokens.access_token()).await;
                    let token = token
                        .inspect_err(|err| log::error!("access token for the SDK failed: {err}"))
                        .ok();
                    cx.update(|cx| player_host::eval(&answer_token(id, token.as_deref()), cx));
                })
                .detach();
            }
            PageMessage::Log { msg } => log::warn!("player page: {msg}"),
            PageMessage::MediaAction { action } => {
                log::debug!("now playing: {action:?}");
                match action {
                    MediaAction::Play => self.play(cx),
                    MediaAction::Pause | MediaAction::Stop => self.pause(cx),
                    MediaAction::NextTrack => self.next(cx),
                    MediaAction::PreviousTrack => self.previous(cx),
                }
            }
        }
    }

    fn on_auth(&mut self, state: AuthState, cx: &mut Context<Self>) {
        log::info!("auth: {}", auth_label(&state));
        let signed_in = matches!(state, AuthState::SignedIn { .. });
        self.auth = state;
        if signed_in && self.started {
            self.input(|p, _| p.on_signed_in(), cx);
        }
        self.maybe_start(cx);
        cx.notify();
    }

    /// Creates the SDK player once the script has loaded and a Listener is signed in.
    fn maybe_start(&mut self, cx: &mut Context<Self>) {
        if self.started || !self.sdk_loaded || !matches!(self.auth, AuthState::SignedIn { .. }) {
            return;
        }
        self.started = true;
        self.refresh_sources(cx);
        let volume = self.player.state().volume;
        player_host::eval(&SdkCommand::Start { volume }.js(), cx);
    }

    fn on_timer(&mut self, cx: &mut Context<Self>) {
        self.timer = None;
        self.input(|p, now| p.tick(now), cx);
    }

    /// Runs one Player input, carries out its effects and tells observers. The one place that
    /// emits [`AppEvent::PlaybackChanged`].
    fn input(
        &mut self,
        f: impl FnOnce(&mut Player, Instant) -> Vec<Effect>,
        cx: &mut Context<Self>,
    ) {
        let revision = self.player.playback_revision();
        let effects = f(&mut self.player, Instant::now());
        self.apply(effects, cx);
        if self.player.playback_revision() != revision {
            cx.emit(AppEvent::PlaybackChanged);
        }
    }

    fn apply(&mut self, effects: Vec<Effect>, cx: &mut Context<Self>) {
        for effect in effects {
            self.run(effect, cx);
        }
        self.arm_timer(cx);
        cx.notify();
    }

    fn run(&mut self, effect: Effect, cx: &mut Context<Self>) {
        match effect {
            Effect::Sdk(command) => {
                log::debug!("player: {command:?}");
                player_host::eval(&command.js(), cx)
            }
            Effect::ReportDevice(id) => self.device_id = id,
            Effect::SaveResume(point) => self.state_file.save_resume_point(point),
            Effect::ClearResume => self.state_file.clear_resume_point(),
            Effect::SaveVolume(v) => self.state_file.save_volume(v),
            Effect::Launch => {
                let resume = self.state_file.resume_point();
                log::info!(
                    "launch: resume point {}",
                    if resume.is_some() { "found" } else { "absent" }
                );
                let effects = self.player.launch(resume);
                for effect in effects {
                    self.run(effect, cx);
                }
            }
            Effect::StartSource {
                source,
                resume,
                launch,
            } => {
                let services = self.services.clone();
                let device_id = self.device_id.clone();
                let user_id = self.user_id();
                self.spawn_input(
                    move || {
                        let (Some(device_id), Some(user_id)) = (device_id, user_id) else {
                            return Err(CommandError::Bridge(BridgeError::new(
                                BridgeErrorCode::NoDevice,
                            )));
                        };
                        start_source(
                            &*services.api,
                            StartSource {
                                source: &source,
                                user_id: &user_id,
                                device_id: &device_id,
                                resume: resume.as_ref(),
                            },
                        )
                        .map_err(command_error)
                    },
                    move |p, result, now| p.on_start_result(launch, result, now),
                    cx,
                );
            }
            Effect::TransferHere => {
                let services = self.services.clone();
                let device_id = self.device_id.clone();
                self.spawn_input(
                    move || match device_id {
                        Some(id) => transfer_here(&*services.api, &id).map_err(command_error),
                        None => Err(CommandError::Bridge(BridgeError::new(
                            BridgeErrorCode::NoDevice,
                        ))),
                    },
                    |p, result, now| match result {
                        Ok(()) => Vec::new(),
                        Err(e) => p.on_command_error(e, now),
                    },
                    cx,
                );
            }
            Effect::PollElsewhere => {
                let services = self.services.clone();
                let device_id = self.device_id.clone();
                self.spawn_input(
                    move || get_playing_elsewhere(&*services.api, device_id.as_deref()),
                    |p, result, _| match result {
                        Ok(info) => p.on_elsewhere(info),
                        Err(err) => {
                            log::error!("polling the other device failed: {err}");
                            Vec::new()
                        }
                    },
                    cx,
                );
            }
            Effect::ReportNotPremium => self.services.tokens.report_not_premium(),
            Effect::RetryAuth => {
                let services = self.services.clone();
                self.spawn_input(
                    move || services.tokens.access_token().map(drop),
                    |p, result, _| match result {
                        Ok(()) => vec![Effect::Sdk(SdkCommand::Connect)],
                        Err(err) => {
                            log::error!("retry after an SDK auth error failed: {err}");
                            p.on_auth_retry_failed()
                        }
                    },
                    cx,
                );
            }
        }
    }

    /// Runs `work` off the main thread, then feeds its result back through `then`.
    fn spawn_input<R: Send + 'static>(
        &self,
        work: impl FnOnce() -> R + Send + 'static,
        then: impl FnOnce(&mut Player, R, Instant) -> Vec<Effect> + 'static,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let result = blocking(work).await;
            let _ = this.update(cx, |m, cx| m.input(|p, now| then(p, result, now), cx));
        })
        .detach();
    }

    /// Wakes the Player at its next deadline. Replacing the task cancels the old wake-up.
    fn arm_timer(&mut self, cx: &mut Context<Self>) {
        let Some(deadline) = self.player.next_deadline() else {
            self.timer = None;
            return;
        };
        if self.timer.as_ref().is_some_and(|(at, _)| *at == deadline) {
            return;
        }
        let wait = deadline.saturating_duration_since(Instant::now());
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |m, cx| m.on_timer(cx));
        });
        self.timer = Some((deadline, task));
    }

    fn user_id(&self) -> Option<String> {
        match &self.auth {
            AuthState::SignedIn { user_id, .. } => Some(user_id.clone()),
            _ => None,
        }
    }
}

fn command_error(err: ApiError) -> CommandError {
    match err {
        ApiError::Bridge(e) => CommandError::Bridge(e),
        other => CommandError::Other(other.to_string()),
    }
}

fn auth_label(state: &AuthState) -> String {
    match state {
        AuthState::SignedOut { reason, .. } => format!("signed-out {reason:?}"),
        AuthState::SigningIn => "signing-in".into(),
        AuthState::SignedIn { .. } => "signed-in".into(),
    }
}

/// Runs a blocking call on its own thread. The auth and Web API clients block for up to 30 s
/// (sign-in for minutes), which would starve GPUI's small background pool.
pub(crate) async fn blocking<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.await.expect("the blocking thread sends before it exits")
}

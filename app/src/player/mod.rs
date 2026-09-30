//! The Player's logic, ported from the Electron renderer's `player.ts`: the state machine, the
//! launch sequence, reconnecting, the "Playing on" poll, Resume Point and volume saving, and the
//! transient message.
//!
//! It does no I/O and owns no timers. Every input takes `now` and returns the [`Effect`]s the
//! caller must carry out; the caller wakes it through [`Player::tick`] at
//! [`Player::next_deadline`]. `AppModel` is that caller.

pub mod format;
pub mod page;
mod schedule;
pub mod view;

use std::sync::Arc;
use std::time::{Duration, Instant};

use slopify_spotify::{
    Artist, BridgeError, BridgeErrorCode, PlayingElsewhere, Resume, ResumePoint, Source,
};

use self::page::{SdkCommand, SdkState};
use self::schedule::{Debounce, PlaybackErrorGate, Throttle};

const RECONNECT: Duration = Duration::from_secs(30);
const ELSEWHERE_POLL: Duration = Duration::from_secs(10);
const TICK: Duration = Duration::from_millis(500);
const RESUME_SAVE: Duration = Duration::from_secs(5);
const VOLUME_SAVE: Duration = Duration::from_millis(300);
const MESSAGE: Duration = Duration::from_secs(5);
// If the resumed track never shows up in a state event the volume must still come back.
const RESUME_TIMEOUT: Duration = Duration::from_secs(15);
// The SDK drops a pause that arrives while the resumed track is still loading.
const RESUME_PAUSE_RETRY: Duration = Duration::from_secs(1);

const ANOTHER_DEVICE: &str = "another device";
const AUTH_FAILED: &str = "Spotify rejected the sign-in.";

#[derive(Debug, Clone, PartialEq)]
pub struct TrackInfo {
    pub id: Option<String>,
    pub uri: String,
    pub name: String,
    pub artists: Vec<Artist>,
    pub album: String,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerState {
    /// False only after `not_ready`. Before the first `ready` the Dropdown shows the empty state.
    pub connected: bool,
    pub track: Option<TrackInfo>,
    pub paused: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
    /// 0..=1, mirrors the SDK volume.
    pub volume: f64,
    pub source: Option<Source>,
    /// Device name while another device holds the session.
    pub elsewhere: Option<String>,
    /// Shown in place of the times, e.g. a playback error.
    pub message: Option<String>,
}

/// Work the Player hands back to its caller.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Sdk(SdkCommand),
    ReportDevice(Option<String>),
    SaveResume(ResumePoint),
    ClearResume,
    SaveVolume(f64),
    /// Answer with [`Player::on_start_result`], passing `launch` back.
    StartSource {
        source: Source,
        resume: Option<Resume>,
        launch: bool,
    },
    /// Answer failures with [`Player::on_command_error`].
    TransferHere,
    /// Answer with [`Player::on_elsewhere`].
    PollElsewhere,
    ReportNotPremium,
    /// Fetch a token, then `Sdk(Connect)`; on failure call [`Player::on_auth_retry_failed`].
    RetryAuth,
    /// Read the Resume Point and pass it to [`Player::launch`].
    Launch,
}

/// A failed Web API command, as the Player needs to see it.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandError {
    Bridge(BridgeError),
    Other(String),
}

impl CommandError {
    pub fn message(&self) -> String {
        match self {
            Self::Bridge(e) => e.message.clone().unwrap_or_else(|| {
                let code = serde_json::to_value(e.code)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default();
                match e.status {
                    Some(status) => format!("Spotify refused ({code} {status})"),
                    None => format!("Spotify refused ({code})"),
                }
            }),
            Self::Other(message) => message.clone(),
        }
    }

    fn is(&self, code: BridgeErrorCode) -> bool {
        matches!(self, Self::Bridge(e) if e.code == code)
    }

    fn status(&self) -> Option<u16> {
        match self {
            Self::Bridge(e) => e.status,
            Self::Other(_) => None,
        }
    }
}

struct PendingResume {
    track_uri: Option<String>,
    volume: f64,
    /// When the last pause went out, answering a state that was still playing.
    paused_at: Option<Instant>,
}

pub struct Player {
    state: PlayerState,
    fatal: Option<String>,
    had_ready: bool,
    auth_retried: bool,
    auth_error: Option<String>,
    reconnect_at: Option<Instant>,
    elsewhere_poll_at: Option<Instant>,
    message_until: Option<Instant>,
    resume_timeout_at: Option<Instant>,
    next_tick: Option<Instant>,
    base_position: u64,
    position_at: Instant,
    pending_resume: Option<PendingResume>,
    error_gate: PlaybackErrorGate,
    known_sources: Arc<Vec<Source>>,
    user_id: Option<String>,
    save_resume: Throttle<ResumePoint>,
    save_volume: Debounce<f64>,
    playback_revision: u64,
    effects: Vec<Effect>,
}

impl Player {
    pub fn new(volume: f64, now: Instant) -> Self {
        Self {
            state: PlayerState {
                connected: true,
                track: None,
                paused: true,
                position_ms: 0,
                duration_ms: 0,
                volume,
                source: None,
                elsewhere: None,
                message: None,
            },
            fatal: None,
            had_ready: false,
            auth_retried: false,
            auth_error: None,
            reconnect_at: None,
            elsewhere_poll_at: None,
            message_until: None,
            resume_timeout_at: None,
            next_tick: None,
            base_position: 0,
            position_at: now,
            pending_resume: None,
            error_gate: PlaybackErrorGate::default(),
            known_sources: Arc::default(),
            user_id: None,
            save_resume: Throttle::new(RESUME_SAVE),
            save_volume: Debounce::new(VOLUME_SAVE),
            playback_revision: 0,
            effects: Vec::new(),
        }
    }

    pub fn state(&self) -> &PlayerState {
        &self.state
    }

    /// Set after the SDK fails twice on init or auth. Shown as the Sign-in State's error line.
    pub fn fatal(&self) -> Option<&str> {
        self.fatal.as_deref()
    }

    /// Bumps whenever the track, paused flag, duration, position anchor or "Playing on" state
    /// changes. The 500 ms position tick does not bump it; the position there is interpolated.
    pub fn playback_revision(&self) -> u64 {
        self.playback_revision
    }

    pub fn set_known_sources(&mut self, sources: Arc<Vec<Source>>, user_id: Option<String>) {
        self.known_sources = sources;
        self.user_id = user_id;
    }

    // Transport

    pub fn toggle_play(&mut self) -> Vec<Effect> {
        if self.state.elsewhere.is_some() {
            self.effects.push(Effect::TransferHere);
        } else if self.state.track.is_some() && self.state.connected {
            self.sdk(SdkCommand::TogglePlay);
        }
        self.take()
    }

    pub fn play(&mut self) -> Vec<Effect> {
        if self.state.elsewhere.is_some() {
            return self.toggle_play();
        }
        self.sdk(SdkCommand::Resume);
        self.take()
    }

    pub fn pause(&mut self) -> Vec<Effect> {
        self.sdk(SdkCommand::Pause);
        self.take()
    }

    pub fn next(&mut self) -> Vec<Effect> {
        if self.state.track.is_some() && self.state.connected && self.state.elsewhere.is_none() {
            self.sdk(SdkCommand::NextTrack);
        }
        self.take()
    }

    pub fn previous(&mut self) -> Vec<Effect> {
        self.sdk(SdkCommand::PreviousTrack);
        self.take()
    }

    pub fn set_volume(&mut self, volume: f64, now: Instant) -> Vec<Effect> {
        let v = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.state.volume = v;
        // A change during the launch sequence is what the Listener wants restored.
        if let Some(pending) = &mut self.pending_resume {
            pending.volume = v;
        }
        self.sdk(SdkCommand::SetVolume(v));
        self.save_volume.call(now, v);
        self.take()
    }

    pub fn start_source(&mut self, source: Source) -> Vec<Effect> {
        self.state.source = Some(source.clone());
        self.effects.push(Effect::StartSource {
            source,
            resume: None,
            launch: false,
        });
        self.take()
    }

    pub fn on_start_result(
        &mut self,
        launch: bool,
        result: Result<(), CommandError>,
        now: Instant,
    ) -> Vec<Effect> {
        match (launch, result) {
            (false, Ok(())) => {}
            (false, Err(e)) => self.show_error(&e, now),
            (true, Ok(())) => {
                if self.pending_resume.is_some() {
                    self.resume_timeout_at = Some(now + RESUME_TIMEOUT);
                }
            }
            (true, Err(e)) => {
                self.restore_volume();
                if e.is(BridgeErrorCode::PlayFailed) && matches!(e.status(), Some(403 | 404)) {
                    self.effects.push(Effect::ClearResume);
                    self.state.source = None;
                } else if e.is(BridgeErrorCode::NoDevice) {
                    self.not_ready(now);
                } else {
                    self.show_error(&e, now);
                }
            }
        }
        self.take()
    }

    pub fn on_command_error(&mut self, error: CommandError, now: Instant) -> Vec<Effect> {
        self.show_error(&error, now);
        self.take()
    }

    // Auth

    /// A fresh sign-in after `expired` gets the launch sequence again once the SDK reconnects.
    pub fn on_signed_in(&mut self) -> Vec<Effect> {
        self.fatal = None;
        self.auth_retried = false;
        if !self.state.connected {
            self.had_ready = false;
        }
        self.take()
    }

    // SDK events

    pub fn on_ready(&mut self, device_id: String) -> Vec<Effect> {
        self.effects.push(Effect::ReportDevice(Some(device_id)));
        self.reconnect_at = None;
        self.state.connected = true;
        if !self.had_ready {
            self.had_ready = true;
            self.effects.push(Effect::Launch);
        }
        self.take()
    }

    pub fn on_not_ready(&mut self, now: Instant) -> Vec<Effect> {
        self.not_ready(now);
        self.take()
    }

    fn not_ready(&mut self, now: Instant) {
        self.effects.push(Effect::ReportDevice(None));
        self.state.connected = false;
        self.reconnect_at.get_or_insert(now + RECONNECT);
    }

    pub fn on_state(&mut self, state: Option<SdkState>, now: Instant) -> Vec<Effect> {
        let Some(s) = state else {
            self.enter_elsewhere(now);
            return self.take();
        };
        self.elsewhere_poll_at = None;
        self.state.elsewhere = None;

        let context_uri = s.context.uri;
        let track = TrackInfo::from(s.track_window.current_track);
        let track_changed = self.state.track.as_ref().map(|t| &t.uri) != Some(&track.uri);
        let uri = track.uri.clone();
        self.state.track = Some(track);
        self.state.paused = s.paused;
        self.state.duration_ms = ms(s.duration);
        self.base_position = ms(s.position);
        self.position_at = now;
        self.state.position_ms = self.base_position;

        if let Some(source) = view::source_for_context(
            context_uri.as_deref(),
            &self.known_sources,
            self.user_id.as_deref(),
        ) {
            self.state.source = Some(source);
        }

        self.playback_revision += 1;
        self.save_resume(track_changed || s.paused, now);
        self.finish_resume(&uri, s.paused, now);
        self.schedule_tick(now);
        self.take()
    }

    pub fn on_elsewhere(&mut self, info: Option<PlayingElsewhere>) -> Vec<Effect> {
        // A poll that lands after this device took the session back is stale.
        if self.elsewhere_poll_at.is_none() {
            return self.take();
        }
        self.state.paused = true;
        self.state.elsewhere = Some(
            info.as_ref()
                .map_or(ANOTHER_DEVICE.into(), |i| i.device_name.clone()),
        );
        if let Some(track) = info.and_then(|i| i.track) {
            self.state.track = Some(TrackInfo {
                id: track.uri.rsplit(':').next().map(str::to_owned),
                uri: track.uri,
                name: track.name,
                artists: track.artists,
                album: track.album,
                image_url: track.image_url,
            });
            self.state.position_ms = 0;
        }
        self.playback_revision += 1;
        self.take()
    }

    /// `initialization_error` or `authentication_error`.
    pub fn on_auth_error(&mut self, message: String) -> Vec<Effect> {
        if self.auth_retried {
            self.fatal = Some(non_empty(message));
        } else {
            self.auth_retried = true;
            self.auth_error = Some(message);
            self.effects.push(Effect::RetryAuth);
        }
        self.take()
    }

    pub fn on_auth_retry_failed(&mut self) -> Vec<Effect> {
        self.fatal = Some(non_empty(self.auth_error.take().unwrap_or_default()));
        self.take()
    }

    pub fn on_account_error(&mut self) -> Vec<Effect> {
        self.effects.push(Effect::ReportNotPremium);
        self.take()
    }

    pub fn on_playback_error(&mut self, message: String, now: Instant) -> Vec<Effect> {
        let sticky = self.error_gate.record(now);
        self.show_message(message, sticky, now);
        self.take()
    }

    // Launch sequence (docs/spec/v1.md, "Launch sequence")

    pub fn launch(&mut self, resume: Option<ResumePoint>) -> Vec<Effect> {
        let Some(resume) = resume else {
            return self.take();
        };
        self.state.source = Some(resume.source.clone());
        self.pending_resume = Some(PendingResume {
            track_uri: resume.track_uri.clone(),
            volume: self.state.volume,
            paused_at: None,
        });
        self.sdk(SdkCommand::SetVolume(0.0));
        self.effects.push(Effect::StartSource {
            source: resume.source,
            resume: Some(Resume {
                track_uri: resume.track_uri,
                position_ms: resume.position_ms,
            }),
            launch: true,
        });
        self.take()
    }

    /// Pauses the resumed track once it plays, and restores the volume once it has paused. A
    /// paused state before any pause went out is the track still loading, not the pause landing.
    fn finish_resume(&mut self, track_uri: &str, paused: bool, now: Instant) {
        let Some(pending) = &mut self.pending_resume else {
            return;
        };
        if pending
            .track_uri
            .as_deref()
            .is_some_and(|uri| uri != track_uri)
        {
            return;
        }
        if !paused {
            if pending
                .paused_at
                .is_none_or(|at| now.duration_since(at) >= RESUME_PAUSE_RETRY)
            {
                pending.paused_at = Some(now);
                self.sdk(SdkCommand::Pause);
            }
        } else if pending.paused_at.is_some() {
            self.restore_volume();
        }
    }

    fn restore_volume(&mut self) {
        let Some(pending) = self.pending_resume.take() else {
            return;
        };
        self.resume_timeout_at = None;
        self.sdk(SdkCommand::SetVolume(pending.volume));
    }

    // Time

    pub fn next_deadline(&self) -> Option<Instant> {
        [
            self.reconnect_at,
            self.elsewhere_poll_at,
            self.message_until,
            self.resume_timeout_at,
            self.next_tick,
            self.save_resume.deadline(),
            self.save_volume.deadline(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    pub fn tick(&mut self, now: Instant) -> Vec<Effect> {
        if self.reconnect_at.is_some_and(|at| at <= now) {
            self.reconnect_at = Some(now + RECONNECT);
            self.sdk(SdkCommand::Connect);
        }
        if self.elsewhere_poll_at.is_some_and(|at| at <= now) {
            self.elsewhere_poll_at = Some(now + ELSEWHERE_POLL);
            self.effects.push(Effect::PollElsewhere);
        }
        if self.message_until.is_some_and(|at| at <= now) {
            self.message_until = None;
            self.state.message = None;
        }
        if self.resume_timeout_at.is_some_and(|at| at <= now) {
            self.restore_volume();
        }
        if self.next_tick.is_some_and(|at| at <= now) {
            self.next_tick = None;
            if self.playing() {
                self.state.position_ms = self.interpolated(now);
                self.schedule_tick(now);
            }
        }
        if let Some(point) = self.save_resume.poll(now) {
            self.effects.push(Effect::SaveResume(point));
        }
        if let Some(v) = self.save_volume.poll(now) {
            self.effects.push(Effect::SaveVolume(v));
        }
        self.take()
    }

    /// Saves where playback is right now and any volume change still waiting, before quit.
    pub fn on_quit(&mut self, now: Instant) -> Vec<Effect> {
        self.save_resume.cancel();
        if let (Some(source), Some(track)) = (&self.state.source, &self.state.track)
            && self.state.elsewhere.is_none()
        {
            let position_ms = if self.playing() {
                self.interpolated(now)
            } else {
                self.state.position_ms
            };
            self.effects.push(Effect::SaveResume(ResumePoint {
                source: source.clone(),
                track_uri: Some(track.uri.clone()),
                position_ms,
            }));
        }
        if let Some(v) = self.save_volume.flush() {
            self.effects.push(Effect::SaveVolume(v));
        }
        self.take()
    }

    // Helpers

    fn enter_elsewhere(&mut self, now: Instant) {
        if self.elsewhere_poll_at.is_some() {
            return;
        }
        self.state.elsewhere = Some(ANOTHER_DEVICE.into());
        self.state.paused = true;
        self.next_tick = None;
        self.playback_revision += 1;
        self.effects.push(Effect::PollElsewhere);
        self.elsewhere_poll_at = Some(now + ELSEWHERE_POLL);
    }

    fn playing(&self) -> bool {
        !self.state.paused && self.state.track.is_some() && self.state.elsewhere.is_none()
    }

    fn interpolated(&self, now: Instant) -> u64 {
        let elapsed = now.duration_since(self.position_at).as_millis() as u64;
        (self.base_position + elapsed).min(self.state.duration_ms)
    }

    fn schedule_tick(&mut self, now: Instant) {
        self.next_tick = self.playing().then(|| now + TICK);
    }

    fn save_resume(&mut self, immediate: bool, now: Instant) {
        let (Some(source), Some(track)) = (&self.state.source, &self.state.track) else {
            return;
        };
        let point = ResumePoint {
            source: source.clone(),
            track_uri: Some(track.uri.clone()),
            position_ms: self.state.position_ms,
        };
        if immediate {
            self.save_resume.cancel();
            self.effects.push(Effect::SaveResume(point));
        } else if let Some(point) = self.save_resume.call(now, point) {
            self.effects.push(Effect::SaveResume(point));
        }
    }

    fn show_error(&mut self, error: &CommandError, now: Instant) {
        self.show_message(error.message(), false, now);
    }

    fn show_message(&mut self, message: String, sticky: bool, now: Instant) {
        if self.error_gate.is_tripped() && self.state.message.is_some() && !sticky {
            return;
        }
        self.state.message = Some(message);
        self.message_until = (!sticky).then(|| now + MESSAGE);
    }

    fn sdk(&mut self, command: SdkCommand) {
        self.effects.push(Effect::Sdk(command));
    }

    fn take(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }
}

fn ms(v: f64) -> u64 {
    if v.is_finite() && v > 0.0 {
        v as u64
    } else {
        0
    }
}

fn non_empty(message: String) -> String {
    if message.is_empty() {
        AUTH_FAILED.into()
    } else {
        message
    }
}

#[cfg(test)]
mod tests;

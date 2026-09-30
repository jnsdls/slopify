use std::time::{Duration, Instant};

use slopify_spotify::{BridgeError, BridgeErrorCode, ElsewhereTrack, PlayingElsewhere};

use super::page::{SdkAlbum, SdkContext, SdkTrack, TrackWindow};
use super::*;

fn ms(t0: Instant, ms: u64) -> Instant {
    t0 + Duration::from_millis(ms)
}

fn liked() -> Source {
    Source::Liked
}

fn sdk_state(uri: &str, paused: bool, position: f64) -> SdkState {
    SdkState {
        paused,
        position,
        duration: 200_000.0,
        context: SdkContext {
            uri: Some("spotify:user:me:collection".into()),
        },
        track_window: TrackWindow {
            current_track: SdkTrack {
                id: None,
                uri: uri.into(),
                name: "Song".into(),
                artists: vec![],
                album: SdkAlbum {
                    name: "Album".into(),
                    images: vec![],
                },
            },
        },
    }
}

fn resume_point() -> ResumePoint {
    ResumePoint {
        source: liked(),
        track_uri: Some("spotify:track:r".into()),
        position_ms: 83_000,
    }
}

fn saves(effects: &[Effect]) -> Vec<&ResumePoint> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SaveResume(p) => Some(p),
            _ => None,
        })
        .collect()
}

/// A Player that has had its first `ready` and a playing state for `uri`.
fn playing(t0: Instant) -> Player {
    let mut p = Player::new(0.5, t0);
    p.set_known_sources(Arc::new(vec![liked()]), Some("me".into()));
    p.on_ready("dev".into());
    p.launch(None);
    p.on_state(Some(sdk_state("spotify:track:a", false, 1000.0)), t0);
    p
}

#[test]
fn first_ready_reports_the_device_and_launches_once() {
    let t0 = Instant::now();
    let mut p = Player::new(0.5, t0);
    assert_eq!(
        p.on_ready("dev".into()),
        [Effect::ReportDevice(Some("dev".into())), Effect::Launch]
    );
    assert_eq!(
        p.on_ready("dev".into()),
        [Effect::ReportDevice(Some("dev".into()))]
    );
}

#[test]
fn launch_mutes_resumes_then_pauses_and_restores_the_volume_on_the_resumed_track() {
    let t0 = Instant::now();
    let mut p = Player::new(0.6, t0);
    p.on_ready("dev".into());
    assert_eq!(
        p.launch(Some(resume_point())),
        [
            Effect::Sdk(SdkCommand::SetVolume(0.0)),
            Effect::StartSource {
                source: liked(),
                resume: Some(Resume {
                    track_uri: Some("spotify:track:r".into()),
                    position_ms: 83_000
                }),
                launch: true,
            }
        ]
    );
    assert_eq!(p.state().source, Some(liked()));
    assert!(p.on_start_result(true, Ok(()), t0).is_empty());

    let pause = Effect::Sdk(SdkCommand::Pause);
    let restore = Effect::Sdk(SdkCommand::SetVolume(0.6));

    // Some other track first: still waiting.
    let effects = p.on_state(Some(sdk_state("spotify:track:x", false, 0.0)), t0);
    assert!(!effects.contains(&pause));
    // Still loading: paused, but nothing has been paused yet, so the volume stays down.
    let effects = p.on_state(Some(sdk_state("spotify:track:r", true, 83_000.0)), t0);
    assert!(!effects.contains(&pause) && !effects.contains(&restore));

    let effects = p.on_state(Some(sdk_state("spotify:track:r", false, 83_000.0)), t0);
    assert!(effects.contains(&pause));
    // The SDK dropped that pause: a second goes out, but not more than once a second.
    let effects = p.on_state(
        Some(sdk_state("spotify:track:r", false, 83_100.0)),
        ms(t0, 100),
    );
    assert!(!effects.contains(&pause));
    let effects = p.on_state(
        Some(sdk_state("spotify:track:r", false, 84_000.0)),
        ms(t0, 1000),
    );
    assert!(effects.contains(&pause));

    let effects = p.on_state(
        Some(sdk_state("spotify:track:r", true, 84_100.0)),
        ms(t0, 1100),
    );
    assert!(effects.contains(&restore));
    assert_eq!(p.next_deadline(), None);
    // Only once.
    let effects = p.on_state(
        Some(sdk_state("spotify:track:r", false, 84_100.0)),
        ms(t0, 3000),
    );
    assert!(!effects.contains(&pause));
}

#[test]
fn launch_restores_the_volume_after_the_timeout_if_the_track_never_shows_up() {
    let t0 = Instant::now();
    let mut p = Player::new(0.6, t0);
    p.launch(Some(resume_point()));
    p.on_start_result(true, Ok(()), t0);
    assert_eq!(p.next_deadline(), Some(ms(t0, 15_000)));
    assert!(p.tick(ms(t0, 14_999)).is_empty());
    assert_eq!(
        p.tick(ms(t0, 15_000)),
        [Effect::Sdk(SdkCommand::SetVolume(0.6))]
    );
    assert_eq!(p.next_deadline(), None);
}

#[test]
fn launch_clears_a_resume_point_whose_playlist_is_gone() {
    let t0 = Instant::now();
    for status in [403, 404] {
        let mut p = Player::new(0.6, t0);
        p.launch(Some(resume_point()));
        let err = CommandError::Bridge(BridgeError::with_status(
            BridgeErrorCode::PlayFailed,
            status,
        ));
        assert_eq!(
            p.on_start_result(true, Err(err), t0),
            [Effect::Sdk(SdkCommand::SetVolume(0.6)), Effect::ClearResume]
        );
        assert_eq!(p.state().source, None);
        assert_eq!(p.state().message, None);
    }
}

#[test]
fn launch_without_a_device_goes_to_reconnecting() {
    let t0 = Instant::now();
    let mut p = Player::new(0.6, t0);
    p.launch(Some(resume_point()));
    let err = CommandError::Bridge(BridgeError::new(BridgeErrorCode::NoDevice));
    let effects = p.on_start_result(true, Err(err), t0);
    assert!(effects.contains(&Effect::ReportDevice(None)));
    assert!(!p.state().connected);
}

#[test]
fn launch_does_nothing_without_a_resume_point() {
    let t0 = Instant::now();
    let mut p = Player::new(0.6, t0);
    assert!(p.launch(None).is_empty());
    assert_eq!(p.state().source, None);
}

#[test]
fn not_ready_reconnects_every_30_s_until_ready() {
    let t0 = Instant::now();
    let mut p = Player::new(0.5, t0);
    p.on_ready("dev".into());
    assert_eq!(p.on_not_ready(t0), [Effect::ReportDevice(None)]);
    assert!(!p.state().connected);
    // A second not_ready does not push the reconnect back.
    p.on_not_ready(ms(t0, 10_000));
    assert_eq!(p.next_deadline(), Some(ms(t0, 30_000)));
    assert_eq!(p.tick(ms(t0, 30_000)), [Effect::Sdk(SdkCommand::Connect)]);
    assert_eq!(p.tick(ms(t0, 60_000)), [Effect::Sdk(SdkCommand::Connect)]);
    p.on_ready("dev".into());
    assert!(p.state().connected);
    assert_eq!(p.next_deadline(), None);
}

#[test]
fn a_null_state_polls_elsewhere_every_10_s_until_a_state_comes_back() {
    let t0 = Instant::now();
    let mut p = playing(t0);
    assert_eq!(p.on_state(None, t0), [Effect::PollElsewhere]);
    assert_eq!(p.state().elsewhere.as_deref(), Some("another device"));
    assert!(p.state().paused);
    assert!(p.on_state(None, t0).is_empty());

    p.on_elsewhere(Some(PlayingElsewhere {
        device_name: "Phone".into(),
        track: Some(ElsewhereTrack {
            uri: "spotify:track:p".into(),
            name: "Other".into(),
            artists: vec![],
            album: "B".into(),
            image_url: None,
        }),
    }));
    assert_eq!(p.state().elsewhere.as_deref(), Some("Phone"));
    let track = p.state().track.as_ref().unwrap();
    assert_eq!(
        (track.id.as_deref(), track.name.as_str()),
        (Some("p"), "Other")
    );

    assert_eq!(p.tick(ms(t0, 10_000)), [Effect::PollElsewhere]);
    p.on_state(
        Some(sdk_state("spotify:track:a", true, 0.0)),
        ms(t0, 12_000),
    );
    assert_eq!(p.state().elsewhere, None);
    // A poll answering late does not bring "Playing on" back.
    p.on_elsewhere(None);
    assert_eq!(p.state().elsewhere, None);
    assert!(p.tick(ms(t0, 20_000)).is_empty());
}

#[test]
fn play_transfers_the_session_back_while_elsewhere_and_next_is_disabled() {
    let t0 = Instant::now();
    let mut p = playing(t0);
    p.on_state(None, t0);
    assert_eq!(p.toggle_play(), [Effect::TransferHere]);
    assert_eq!(p.play(), [Effect::TransferHere]);
    assert!(p.next().is_empty());
}

#[test]
fn transport_is_disabled_while_empty_or_reconnecting() {
    let t0 = Instant::now();
    let mut p = Player::new(0.5, t0);
    assert!(p.toggle_play().is_empty());
    assert!(p.next().is_empty());
    let mut p = playing(t0);
    assert_eq!(p.toggle_play(), [Effect::Sdk(SdkCommand::TogglePlay)]);
    assert_eq!(p.next(), [Effect::Sdk(SdkCommand::NextTrack)]);
    p.on_not_ready(t0);
    assert!(p.toggle_play().is_empty());
    assert!(p.next().is_empty());
}

#[test]
fn position_ticks_every_500_ms_while_playing_and_stops_at_the_duration() {
    let t0 = Instant::now();
    let mut p = playing(t0);
    assert_eq!(p.state().position_ms, 1000);
    p.tick(ms(t0, 500));
    assert_eq!(p.state().position_ms, 1500);
    p.tick(ms(t0, 1000));
    assert_eq!(p.state().position_ms, 2000);
    p.tick(ms(t0, 500_000));
    assert_eq!(p.state().position_ms, 200_000);

    p.on_state(Some(sdk_state("spotify:track:a", true, 5000.0)), t0);
    p.tick(ms(t0, 600_000));
    assert_eq!(p.state().position_ms, 5000);
}

#[test]
fn resume_saves_at_once_on_track_change_and_pause_and_throttles_while_playing() {
    let t0 = Instant::now();
    let mut p = Player::new(0.5, t0);
    p.set_known_sources(Arc::new(vec![liked()]), Some("me".into()));
    let effects = p.on_state(Some(sdk_state("spotify:track:a", false, 0.0)), t0);
    assert_eq!(
        saves(&effects)[0].track_uri.as_deref(),
        Some("spotify:track:a")
    );
    assert_eq!(saves(&effects)[0].source, liked());

    // Same track, playing: the first goes through, the rest wait for the window.
    let effects = p.on_state(
        Some(sdk_state("spotify:track:a", false, 1000.0)),
        ms(t0, 1000),
    );
    assert_eq!(saves(&effects).len(), 1);
    let effects = p.on_state(
        Some(sdk_state("spotify:track:a", false, 2000.0)),
        ms(t0, 2000),
    );
    assert!(saves(&effects).is_empty());
    let effects = p.tick(ms(t0, 6000));
    assert_eq!(saves(&effects)[0].position_ms, 2000);

    let effects = p.on_state(
        Some(sdk_state("spotify:track:a", true, 2500.0)),
        ms(t0, 6100),
    );
    assert_eq!(saves(&effects)[0].position_ms, 2500);
}

#[test]
fn nothing_is_saved_without_a_source() {
    let t0 = Instant::now();
    let mut p = Player::new(0.5, t0);
    let effects = p.on_state(Some(sdk_state("spotify:track:a", true, 0.0)), t0);
    assert!(saves(&effects).is_empty());
}

#[test]
fn volume_goes_to_the_sdk_at_once_and_is_saved_300_ms_after_the_last_change() {
    let t0 = Instant::now();
    let mut p = Player::new(0.5, t0);
    assert_eq!(
        p.set_volume(0.2, t0),
        [Effect::Sdk(SdkCommand::SetVolume(0.2))]
    );
    p.set_volume(1.7, ms(t0, 100));
    assert_eq!(p.state().volume, 1.0);
    assert!(p.tick(ms(t0, 399)).is_empty());
    assert_eq!(p.tick(ms(t0, 400)), [Effect::SaveVolume(1.0)]);
}

#[test]
fn a_command_error_shows_for_5_s() {
    let t0 = Instant::now();
    let mut p = Player::new(0.5, t0);
    let err = CommandError::Bridge(BridgeError::with_status(BridgeErrorCode::PlayFailed, 500));
    p.on_start_result(false, Err(err), t0);
    assert_eq!(
        p.state().message.as_deref(),
        Some("Spotify refused (play-failed 500)")
    );
    p.tick(ms(t0, 5000));
    assert_eq!(p.state().message, None);
}

#[test]
fn playback_errors_stick_after_three_in_a_minute() {
    let t0 = Instant::now();
    let mut p = Player::new(0.5, t0);
    p.on_playback_error("one".into(), t0);
    p.on_playback_error("two".into(), ms(t0, 1000));
    assert_eq!(p.next_deadline(), Some(ms(t0, 6000)));
    p.on_playback_error("three".into(), ms(t0, 2000));
    assert_eq!(p.state().message.as_deref(), Some("three"));
    assert_eq!(p.next_deadline(), None);
    // Once tripped, a transient error cannot replace the sticky one.
    p.on_command_error(CommandError::Other("later".into()), ms(t0, 3000));
    assert_eq!(p.state().message.as_deref(), Some("three"));
}

#[test]
fn auth_errors_retry_once_then_turn_fatal() {
    let t0 = Instant::now();
    let mut p = Player::new(0.5, t0);
    assert_eq!(p.on_auth_error("bad".into()), [Effect::RetryAuth]);
    assert_eq!(p.fatal(), None);
    assert!(p.on_auth_error(String::new()).is_empty());
    assert_eq!(p.fatal(), Some("Spotify rejected the sign-in."));

    p.on_signed_in();
    assert_eq!(p.fatal(), None);
    assert_eq!(p.on_auth_error("bad".into()), [Effect::RetryAuth]);
    p.on_auth_retry_failed();
    assert_eq!(p.fatal(), Some("bad"));
}

#[test]
fn a_sign_in_after_a_disconnect_runs_the_launch_sequence_again() {
    let t0 = Instant::now();
    let mut p = Player::new(0.5, t0);
    p.on_ready("dev".into());
    p.on_not_ready(t0);
    p.on_signed_in();
    assert!(p.on_ready("dev".into()).contains(&Effect::Launch));
}

#[test]
fn account_error_reports_not_premium() {
    let mut p = Player::new(0.5, Instant::now());
    assert_eq!(p.on_account_error(), [Effect::ReportNotPremium]);
}

#[test]
fn start_source_sets_the_source_before_the_call() {
    let mut p = Player::new(0.5, Instant::now());
    assert_eq!(
        p.start_source(liked()),
        [Effect::StartSource {
            source: liked(),
            resume: None,
            launch: false
        }]
    );
    assert_eq!(p.state().source, Some(liked()));
}

#[test]
fn quit_saves_the_interpolated_position_and_a_pending_volume() {
    let t0 = Instant::now();
    let mut p = playing(t0);
    p.set_volume(0.3, t0);
    let effects = p.on_quit(ms(t0, 2500));
    assert_eq!(saves(&effects)[0].position_ms, 3500);
    assert!(effects.contains(&Effect::SaveVolume(0.3)));
}

#[test]
fn the_playback_revision_moves_on_state_events_but_not_on_ticks() {
    let t0 = Instant::now();
    let mut p = playing(t0);
    let rev = p.playback_revision();
    p.tick(ms(t0, 500));
    assert_eq!(p.playback_revision(), rev);
    p.on_state(Some(sdk_state("spotify:track:a", true, 600.0)), ms(t0, 600));
    assert!(p.playback_revision() > rev);
}

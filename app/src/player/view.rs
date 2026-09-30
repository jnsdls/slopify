//! The pure rules the Dropdown derives its view from. Ported from the Electron renderer's
//! `model.ts`.

use slopify_auth::{AuthState, SignedOutReason};
use slopify_spotify::{BridgeErrorCode, Source};

use super::PlayerState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Empty,
    Playing,
    Paused,
    Elsewhere,
    Reconnecting,
}

pub fn status_of(s: &PlayerState) -> Status {
    if !s.connected {
        Status::Reconnecting
    } else if s.elsewhere.is_some() {
        Status::Elsewhere
    } else if s.track.is_none() {
        Status::Empty
    } else if s.paused {
        Status::Paused
    } else {
        Status::Playing
    }
}

/// What the progress area shows in place of the times, and whether it reads as an error.
pub fn progress_line(s: &PlayerState) -> Option<(String, bool)> {
    if let Some(message) = &s.message {
        return Some((message.clone(), true));
    }
    match (status_of(s), &s.elsewhere) {
        (Status::Elsewhere, Some(device)) => Some((format!("Playing on {device}"), false)),
        (Status::Reconnecting, _) => Some(("Reconnecting".into(), false)),
        _ => None,
    }
}

/// Which controls take input. Play doubles as "take it back" while another device plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Controls {
    pub play: bool,
    pub next: bool,
    pub volume: bool,
}

pub fn controls(status: Status) -> Controls {
    Controls {
        play: !matches!(status, Status::Empty | Status::Reconnecting),
        next: matches!(status, Status::Playing | Status::Paused),
        volume: matches!(status, Status::Playing | Status::Paused),
    }
}

/// How much of the progress bar is filled, 0..=1.
pub fn progress_fraction(position_ms: u64, duration_ms: u64) -> f32 {
    if duration_ms == 0 {
        0.0
    } else {
        (position_ms as f64 / duration_ms as f64).min(1.0) as f32
    }
}

/// What the Sign-in State shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignInView {
    pub line: String,
    /// The button is disabled and reads "Waiting for Spotify".
    pub waiting: bool,
}

pub fn sign_in_line(reason: SignedOutReason, detail: Option<&str>) -> String {
    match reason {
        SignedOutReason::FirstRun => "Sign in to start playing.".into(),
        SignedOutReason::Expired => "Spotify signed you out. Sign in again to keep going.".into(),
        SignedOutReason::NotPremium => "slopify needs a Premium account.".into(),
        SignedOutReason::Error => detail.unwrap_or("Something went wrong.").into(),
    }
}

/// `None` means the player layout. A fatal Player error wins over the auth state.
pub fn sign_in_view(auth: &AuthState, fatal: Option<&str>) -> Option<SignInView> {
    if let Some(line) = fatal {
        return Some(SignInView {
            line: line.into(),
            waiting: false,
        });
    }
    match auth {
        AuthState::SigningIn => Some(SignInView {
            line: "Sign in to start playing.".into(),
            waiting: true,
        }),
        AuthState::SignedOut { reason, detail } => Some(SignInView {
            line: sign_in_line(*reason, detail.as_deref()),
            waiting: false,
        }),
        AuthState::SignedIn { .. } => None,
    }
}

pub fn paste_error_line(code: Option<BridgeErrorCode>) -> &'static str {
    match code {
        Some(BridgeErrorCode::BadLink) => "That's not a playlist link",
        Some(BridgeErrorCode::NotFound | BridgeErrorCode::Forbidden) => {
            "Spotify won't share that playlist with this app"
        }
        _ => "Couldn't open that playlist",
    }
}

pub fn source_name(source: Option<&Source>) -> &str {
    match source {
        None => "Choose a source",
        Some(Source::Liked) => "Liked Songs",
        Some(Source::Playlist { name, .. }) => name,
    }
}

pub fn same_source(a: Option<&Source>, b: Option<&Source>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Source::Liked), Some(Source::Liked)) => true,
        (Some(Source::Playlist { id: a, .. }), Some(Source::Playlist { id: b, .. })) => a == b,
        _ => false,
    }
}

/// Which known Source a context uri belongs to, so the "Playing from" row follows a takeover
/// from another device.
pub fn source_for_context(
    uri: Option<&str>,
    sources: &[Source],
    user_id: Option<&str>,
) -> Option<Source> {
    let uri = uri?;
    if let Some(user_id) = user_id
        && uri == format!("spotify:user:{user_id}:collection")
    {
        return Some(Source::Liked);
    }
    sources
        .iter()
        .find(|s| matches!(s, Source::Playlist { uri: u, .. } if u == uri))
        .cloned()
}

/// Rows in the Picker: the fetched list, plus the current Pasted Playlist, which is never in it.
pub fn picker_rows(sources: &[Source], current: Option<&Source>) -> Vec<Source> {
    let mut rows = if sources.is_empty() {
        vec![Source::Liked]
    } else {
        sources.to_vec()
    };
    if let Some(current @ Source::Playlist { pasted: true, .. }) = current
        && !rows.iter().any(|s| same_source(Some(s), Some(current)))
    {
        rows.push(current.clone());
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::TrackInfo;

    fn track() -> TrackInfo {
        TrackInfo {
            id: Some("t".into()),
            uri: "spotify:track:t".into(),
            name: "Song".into(),
            artists: vec![],
            album: "Album".into(),
            image_url: None,
        }
    }

    fn state() -> PlayerState {
        PlayerState {
            connected: true,
            track: None,
            paused: true,
            position_ms: 0,
            duration_ms: 0,
            volume: 0.5,
            source: None,
            elsewhere: None,
            message: None,
        }
    }

    fn playlist(id: &str, pasted: bool) -> Source {
        Source::Playlist {
            id: id.into(),
            uri: format!("spotify:playlist:{id}"),
            name: id.into(),
            image_url: None,
            pasted,
        }
    }

    #[test]
    fn orders_reconnecting_over_elsewhere_over_empty() {
        let s = PlayerState {
            connected: false,
            elsewhere: Some("Phone".into()),
            track: Some(track()),
            ..state()
        };
        assert_eq!(status_of(&s), Status::Reconnecting);
        let s = PlayerState {
            elsewhere: Some("Phone".into()),
            track: Some(track()),
            ..state()
        };
        assert_eq!(status_of(&s), Status::Elsewhere);
        assert_eq!(status_of(&state()), Status::Empty);
        let s = PlayerState {
            track: Some(track()),
            paused: false,
            ..state()
        };
        assert_eq!(status_of(&s), Status::Playing);
        let s = PlayerState {
            track: Some(track()),
            ..state()
        };
        assert_eq!(status_of(&s), Status::Paused);
    }

    fn signed_out(reason: SignedOutReason, detail: Option<&str>) -> AuthState {
        AuthState::SignedOut {
            reason,
            detail: detail.map(Into::into),
        }
    }

    #[test]
    fn maps_every_reason_to_its_line() {
        let line = |auth| sign_in_view(&auth, None).unwrap().line;
        assert_eq!(
            line(signed_out(SignedOutReason::FirstRun, None)),
            "Sign in to start playing."
        );
        assert_eq!(
            line(signed_out(SignedOutReason::Expired, None)),
            "Spotify signed you out. Sign in again to keep going."
        );
        assert_eq!(
            line(signed_out(SignedOutReason::NotPremium, None)),
            "slopify needs a Premium account."
        );
        assert_eq!(
            line(signed_out(
                SignedOutReason::Error,
                Some("Port 8888 is in use")
            )),
            "Port 8888 is in use"
        );
    }

    #[test]
    fn disables_the_button_while_signing_in() {
        assert!(sign_in_view(&AuthState::SigningIn, None).unwrap().waiting);
    }

    #[test]
    fn is_none_when_signed_in_unless_the_player_hit_a_fatal_error() {
        let signed_in = AuthState::SignedIn {
            display_name: "j".into(),
            user_id: "u".into(),
        };
        assert_eq!(sign_in_view(&signed_in, None), None);
        assert_eq!(
            sign_in_view(&signed_in, Some("Authentication failed")),
            Some(SignInView {
                line: "Authentication failed".into(),
                waiting: false
            })
        );
    }

    #[test]
    fn has_one_paste_line_for_bad_links_and_one_for_hidden_playlists() {
        assert_eq!(
            paste_error_line(Some(BridgeErrorCode::BadLink)),
            "That's not a playlist link"
        );
        for code in [BridgeErrorCode::NotFound, BridgeErrorCode::Forbidden] {
            assert_eq!(
                paste_error_line(Some(code)),
                "Spotify won't share that playlist with this app"
            );
        }
        assert_eq!(paste_error_line(None), "Couldn't open that playlist");
    }

    #[test]
    fn compares_sources_by_kind_and_id() {
        let liked = Source::Liked;
        assert!(same_source(Some(&liked), Some(&liked)));
        assert!(same_source(
            Some(&playlist("a", false)),
            Some(&playlist("a", true))
        ));
        assert!(!same_source(
            Some(&playlist("a", false)),
            Some(&playlist("b", false))
        ));
        assert!(!same_source(Some(&playlist("a", false)), Some(&liked)));
        assert!(same_source(None, None));
        assert!(!same_source(None, Some(&playlist("a", false))));
    }

    #[test]
    fn finds_playlists_by_uri_and_liked_songs_by_the_collection_uri() {
        let sources = [Source::Liked, playlist("a", false)];
        assert_eq!(
            source_for_context(Some("spotify:playlist:a"), &sources, Some("me")),
            Some(playlist("a", false))
        );
        assert_eq!(
            source_for_context(Some("spotify:user:me:collection"), &sources, Some("me")),
            Some(Source::Liked)
        );
    }

    #[test]
    fn returns_none_for_unknown_or_missing_contexts() {
        let sources = [Source::Liked, playlist("a", false)];
        assert_eq!(
            source_for_context(Some("spotify:playlist:zzz"), &sources, Some("me")),
            None
        );
        assert_eq!(source_for_context(None, &sources, Some("me")), None);
        assert_eq!(
            source_for_context(Some("spotify:user:other:collection"), &sources, Some("me")),
            None
        );
    }

    #[test]
    fn appends_the_current_pasted_playlist_without_keeping_it() {
        let sources = [Source::Liked, playlist("a", false)];
        let pasted = playlist("p", true);
        let mut expected = sources.to_vec();
        expected.push(pasted.clone());
        assert_eq!(picker_rows(&sources, Some(&pasted)), expected);
        assert_eq!(
            picker_rows(&sources, Some(&playlist("a", false))),
            sources.to_vec()
        );
    }

    #[test]
    fn shows_at_least_liked_songs_before_the_list_has_loaded() {
        assert_eq!(picker_rows(&[], None), vec![Source::Liked]);
    }

    #[test]
    fn shows_the_message_over_the_other_device_over_reconnecting() {
        let s = PlayerState {
            elsewhere: Some("Phone".into()),
            message: Some("Couldn't play that".into()),
            ..state()
        };
        assert_eq!(progress_line(&s), Some(("Couldn't play that".into(), true)));
        let s = PlayerState {
            elsewhere: Some("Phone".into()),
            ..state()
        };
        assert_eq!(progress_line(&s), Some(("Playing on Phone".into(), false)));
        let s = PlayerState {
            connected: false,
            elsewhere: Some("Phone".into()),
            ..state()
        };
        assert_eq!(progress_line(&s), Some(("Reconnecting".into(), false)));
        assert_eq!(progress_line(&state()), None);
    }

    #[test]
    fn enables_controls_by_status() {
        let c = |play, next, volume| Controls { play, next, volume };
        assert_eq!(controls(Status::Empty), c(false, false, false));
        assert_eq!(controls(Status::Reconnecting), c(false, false, false));
        assert_eq!(controls(Status::Elsewhere), c(true, false, false));
        assert_eq!(controls(Status::Paused), c(true, true, true));
        assert_eq!(controls(Status::Playing), c(true, true, true));
    }

    #[test]
    fn fills_the_progress_bar_up_to_full() {
        assert_eq!(progress_fraction(0, 0), 0.0);
        assert_eq!(progress_fraction(50, 200), 0.25);
        assert_eq!(progress_fraction(300, 200), 1.0);
    }

    #[test]
    fn names_sources() {
        assert_eq!(source_name(None), "Choose a source");
        assert_eq!(source_name(Some(&Source::Liked)), "Liked Songs");
        assert_eq!(source_name(Some(&playlist("Mix", false))), "Mix");
    }
}

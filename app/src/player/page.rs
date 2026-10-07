//! The wire between Rust and `player.html`: messages the page posts, and the JavaScript Rust runs
//! in it. The page holds no state beyond the `Spotify.Player` itself.

use serde::{Deserialize, Serialize};
use slopify_spotify::Artist;

use super::TrackInfo;
use super::format::{Image, largest_image};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "t", rename_all = "kebab-case")]
pub enum PageMessage {
    /// `onSpotifyWebPlaybackSDKReady` fired; `slopify.start` can run.
    SdkLoaded,
    #[serde(rename_all = "camelCase")]
    Ready {
        device_id: String,
    },
    NotReady,
    State {
        state: Option<SdkState>,
    },
    /// Every SDK error event, plus `autoplay_failed` with an empty message.
    Error {
        event: String,
        #[serde(default)]
        message: String,
    },
    /// The SDK's `getOAuthToken`; answer with [`answer_token`].
    Token {
        id: u64,
    },
    Log {
        msg: String,
    },
    /// A media key, AirPods or Control Center, through the SDK iframe's media session.
    MediaAction {
        action: MediaAction,
    },
}

/// The `navigator.mediaSession` actions `media_session.js` handles.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaAction {
    Play,
    Pause,
    Stop,
    NextTrack,
    PreviousTrack,
}

impl PageMessage {
    pub fn parse(raw: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(raw)
    }
}

/// The part of the SDK's `PlaybackState` slopify reads.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SdkState {
    pub paused: bool,
    pub position: f64,
    pub duration: f64,
    pub context: SdkContext,
    pub track_window: TrackWindow,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SdkContext {
    pub uri: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TrackWindow {
    pub current_track: SdkTrack,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SdkTrack {
    pub id: Option<String>,
    pub uri: String,
    pub name: String,
    #[serde(default)]
    pub artists: Vec<Artist>,
    pub album: SdkAlbum,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SdkAlbum {
    pub name: String,
    #[serde(default)]
    pub images: Vec<Image>,
}

impl From<SdkTrack> for TrackInfo {
    fn from(t: SdkTrack) -> Self {
        Self {
            image_url: largest_image(&t.album.images).map(|i| i.url.clone()),
            id: t.id,
            uri: t.uri,
            name: t.name,
            artists: t.artists,
            album: t.album.name,
        }
    }
}

/// What the Player asks the SDK to do. Failures come back as `log` messages.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SdkCommand {
    /// Creates the `Spotify.Player` and connects it.
    Start {
        volume: f64,
    },
    Connect,
    TogglePlay,
    Resume,
    Pause,
    NextTrack,
    PreviousTrack,
    SetVolume(f64),
}

impl SdkCommand {
    pub fn js(self) -> String {
        match self {
            Self::Start { volume } => format!("slopify.start({})", number(volume)),
            Self::Connect => "slopify.connect()".into(),
            Self::TogglePlay => "slopify.togglePlay()".into(),
            Self::Resume => "slopify.resume()".into(),
            Self::Pause => "slopify.pause()".into(),
            Self::NextTrack => "slopify.nextTrack()".into(),
            Self::PreviousTrack => "slopify.previousTrack()".into(),
            Self::SetVolume(v) => format!("slopify.setVolume({})", number(v)),
        }
    }
}

fn number(v: f64) -> String {
    if v.is_finite() {
        v.clamp(0.0, 1.0).to_string()
    } else {
        "0".into()
    }
}

/// Hands an access token to the `getOAuthToken` callback waiting on `id`. `None` drops the
/// callback, which leaves the SDK to report its own error.
pub fn answer_token(id: u64, token: Option<&str>) -> String {
    let token = serde_json::to_string(&token).expect("strings serialize");
    format!("slopify.token({id}, {token})")
}

/// A `MediaMetadataInit` for Now Playing.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub artwork: Vec<Artwork>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Artwork {
    pub src: String,
}

/// Sets Now Playing's metadata. `None` clears it.
pub fn now_playing(metadata: Option<&NowPlaying>) -> String {
    let metadata = serde_json::to_string(&metadata).expect("metadata serializes");
    format!("slopify.nowPlaying({metadata})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ready_and_not_ready() {
        assert_eq!(
            PageMessage::parse(r#"{"t":"ready","deviceId":"d1"}"#).unwrap(),
            PageMessage::Ready {
                device_id: "d1".into()
            }
        );
        assert_eq!(
            PageMessage::parse(r#"{"t":"not-ready"}"#).unwrap(),
            PageMessage::NotReady
        );
        assert_eq!(
            PageMessage::parse(r#"{"t":"sdk-loaded"}"#).unwrap(),
            PageMessage::SdkLoaded
        );
    }

    #[test]
    fn parses_media_actions() {
        assert_eq!(
            PageMessage::parse(r#"{"t":"media-action","action":"nexttrack"}"#).unwrap(),
            PageMessage::MediaAction {
                action: MediaAction::NextTrack
            }
        );
        assert_eq!(
            PageMessage::parse(r#"{"t":"media-action","action":"previoustrack"}"#).unwrap(),
            PageMessage::MediaAction {
                action: MediaAction::PreviousTrack
            }
        );
    }

    #[test]
    fn now_playing_passes_metadata_or_null() {
        let metadata = NowPlaying {
            title: "Song \"1\"".into(),
            artist: "A".into(),
            album: "Album".into(),
            artwork: vec![Artwork { src: "big".into() }],
        };
        assert_eq!(
            now_playing(Some(&metadata)),
            r#"slopify.nowPlaying({"title":"Song \"1\"","artist":"A","album":"Album","artwork":[{"src":"big"}]})"#
        );
        assert_eq!(now_playing(None), "slopify.nowPlaying(null)");
    }

    #[test]
    fn parses_a_playback_state_and_picks_the_largest_artwork() {
        let raw = r#"{"t":"state","state":{"paused":false,"position":1234.5,"duration":200000,
            "context":{"uri":"spotify:playlist:abc","metadata":{}},
            "track_window":{"current_track":{"id":"x","uri":"spotify:track:x","name":"Song",
                "artists":[{"name":"A","uri":"spotify:artist:a"}],
                "album":{"name":"Album","uri":"spotify:album:b","images":[
                    {"url":"small","width":64,"height":64},{"url":"big","width":640,"height":640}]},
                "duration_ms":200000},
                "previous_tracks":[],"next_tracks":[]}}}"#;
        let PageMessage::State { state: Some(state) } = PageMessage::parse(raw).unwrap() else {
            panic!("not a state");
        };
        assert!(!state.paused);
        assert_eq!(state.position, 1234.5);
        assert_eq!(state.context.uri.as_deref(), Some("spotify:playlist:abc"));
        let track = TrackInfo::from(state.track_window.current_track);
        assert_eq!(track.image_url.as_deref(), Some("big"));
        assert_eq!(track.artists[0].name, "A");
        assert_eq!(track.album, "Album");
    }

    #[test]
    fn parses_a_null_state_as_elsewhere() {
        assert_eq!(
            PageMessage::parse(r#"{"t":"state","state":null}"#).unwrap(),
            PageMessage::State { state: None }
        );
    }

    #[test]
    fn parses_errors_and_token_requests() {
        assert_eq!(
            PageMessage::parse(r#"{"t":"error","event":"playback_error","message":"boom"}"#)
                .unwrap(),
            PageMessage::Error {
                event: "playback_error".into(),
                message: "boom".into()
            }
        );
        assert_eq!(
            PageMessage::parse(r#"{"t":"token","id":7}"#).unwrap(),
            PageMessage::Token { id: 7 }
        );
    }

    #[test]
    fn writes_commands_as_calls_on_the_page_shim() {
        assert_eq!(SdkCommand::Start { volume: 0.5 }.js(), "slopify.start(0.5)");
        assert_eq!(SdkCommand::SetVolume(1.0).js(), "slopify.setVolume(1)");
        assert_eq!(SdkCommand::SetVolume(f64::NAN).js(), "slopify.setVolume(0)");
    }

    #[test]
    fn quotes_the_token_as_a_js_string() {
        assert_eq!(answer_token(3, Some("a\"b")), r#"slopify.token(3, "a\"b")"#);
        assert_eq!(answer_token(4, None), "slopify.token(4, null)");
    }
}

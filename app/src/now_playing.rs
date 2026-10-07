//! Now Playing: the Player's track, artist, album and artwork go to the media session of the
//! SDK's iframe, which owns WebKit's Now Playing entry. WebKit reads the times and the playback
//! rate from the SDK's media element itself. The media keys, AirPods and Control Center come back
//! through the same session as `PageMessage::MediaAction`, which `AppModel` handles.

use gpui::{App, Entity, Global};

use crate::app_model::{AppEvent, AppModel};
use crate::player::PlayerState;
use crate::player::page::{self, Artwork, NowPlaying};
use crate::player_host;

/// The metadata last sent, so the page only hears about changes.
#[derive(Default)]
struct Sent(Option<NowPlaying>);

impl Global for Sent {}

/// Mirrors `model`'s playback into Now Playing from here on.
pub fn init(model: Entity<AppModel>, cx: &mut App) {
    cx.set_global(Sent::default());
    cx.subscribe(&model, |model, event, cx| match event {
        AppEvent::PlaybackChanged => {
            let metadata = metadata_of(model.read(cx).player());
            if cx.global::<Sent>().0 != metadata {
                player_host::eval(&page::now_playing(metadata.as_ref()), cx);
                cx.set_global(Sent(metadata));
            }
        }
    })
    .detach();
}

/// `None` clears the metadata: nothing is loaded, or another device holds the session.
fn metadata_of(s: &PlayerState) -> Option<NowPlaying> {
    if s.elsewhere.is_some() {
        return None;
    }
    let track = s.track.as_ref()?;
    Some(NowPlaying {
        title: track.name.clone(),
        artist: track
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        album: track.album.clone(),
        artwork: track
            .image_url
            .iter()
            .map(|src| Artwork { src: src.clone() })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use slopify_spotify::Artist;

    use super::*;
    use crate::player::TrackInfo;

    fn playing() -> PlayerState {
        PlayerState {
            connected: true,
            track: Some(TrackInfo {
                id: Some("t".into()),
                uri: "spotify:track:t".into(),
                name: "Song".into(),
                artists: vec![
                    Artist {
                        name: "A".into(),
                        uri: "spotify:artist:a".into(),
                    },
                    Artist {
                        name: "B".into(),
                        uri: "spotify:artist:b".into(),
                    },
                ],
                album: "Album".into(),
                image_url: Some("https://i.scdn.co/image/x".into()),
            }),
            paused: false,
            position_ms: 61_500,
            duration_ms: 200_000,
            volume: 0.2,
            source: None,
            elsewhere: None,
            message: None,
        }
    }

    #[test]
    fn carries_the_track_and_artwork() {
        assert_eq!(
            metadata_of(&playing()),
            Some(NowPlaying {
                title: "Song".into(),
                artist: "A, B".into(),
                album: "Album".into(),
                artwork: vec![Artwork {
                    src: "https://i.scdn.co/image/x".into()
                }],
            })
        );
    }

    #[test]
    fn no_image_means_no_artwork() {
        let mut s = playing();
        s.track.as_mut().unwrap().image_url = None;
        assert_eq!(metadata_of(&s).unwrap().artwork, []);
    }

    #[test]
    fn clears_without_a_track_or_while_playing_elsewhere() {
        let empty = PlayerState {
            track: None,
            ..playing()
        };
        assert_eq!(metadata_of(&empty), None);
        let elsewhere = PlayerState {
            elsewhere: Some("Phone".into()),
            ..playing()
        };
        assert_eq!(metadata_of(&elsewhere), None);
    }
}

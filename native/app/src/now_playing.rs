//! Now Playing and the media keys through MediaPlayer.framework: the Player's track, times and
//! artwork go to `MPNowPlayingInfoCenter`, and `MPRemoteCommandCenter`'s play, pause, toggle,
//! next and previous come back as `AppModel` actions. The keyboard's media keys, AirPods and
//! Control Center all arrive through the command center.
//!
//! WebKit would register a second, bare entry for the SDK's media element; `player_host` keeps
//! it out.

use std::ptr::NonNull;
use std::time::Duration;

use block2::RcBlock;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Entity, Global};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AllocAnyThread, MainThreadMarker};
use objc2_app_kit::NSImage;
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSData, NSDictionary, NSNumber, NSString};
use objc2_media_player::{
    MPMediaItemArtwork, MPMediaItemPropertyAlbumTitle, MPMediaItemPropertyArtist,
    MPMediaItemPropertyArtwork, MPMediaItemPropertyPlaybackDuration, MPMediaItemPropertyTitle,
    MPNowPlayingInfoCenter, MPNowPlayingInfoPropertyElapsedPlaybackTime,
    MPNowPlayingInfoPropertyPlaybackRate, MPNowPlayingPlaybackState, MPRemoteCommand,
    MPRemoteCommandCenter, MPRemoteCommandEvent, MPRemoteCommandHandlerStatus,
};

use crate::app_model::{AppEvent, AppModel, blocking};
use crate::player::PlayerState;

const ARTWORK_TIMEOUT: Duration = Duration::from_secs(15);
const ARTWORK_LIMIT: u64 = 5 * 1024 * 1024;

/// What Now Playing shows.
#[derive(Debug, Clone, PartialEq)]
struct Info {
    title: String,
    artist: String,
    album: String,
    image_url: Option<String>,
    duration_s: f64,
    elapsed_s: f64,
    playing: bool,
}

/// `None` clears the entry: nothing is loaded, or another device holds the session.
fn info_of(s: &PlayerState) -> Option<Info> {
    if s.elsewhere.is_some() {
        return None;
    }
    let track = s.track.as_ref()?;
    Some(Info {
        title: track.name.clone(),
        artist: track
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        album: track.album.clone(),
        image_url: track.image_url.clone(),
        duration_s: s.duration_ms as f64 / 1000.0,
        elapsed_s: s.position_ms.min(s.duration_ms) as f64 / 1000.0,
        playing: s.connected && !s.paused,
    })
}

#[derive(Debug, Clone, Copy)]
enum Command {
    Play,
    Pause,
    TogglePlay,
    Next,
    Previous,
}

#[derive(Default)]
struct NowPlaying {
    info: Option<Info>,
    /// The URL `artwork` was fetched from, or is being fetched from while `artwork` is `None`.
    artwork_url: Option<String>,
    artwork: Option<Retained<MPMediaItemArtwork>>,
}

impl Global for NowPlaying {}

/// Takes over the media commands and mirrors `model`'s playback into Now Playing from here on.
pub fn init(model: Entity<AppModel>, cx: &mut App) {
    cx.set_global(NowPlaying::default());
    register_commands(model.clone(), cx);
    cx.subscribe(&model, |model, event, cx| match event {
        AppEvent::PlaybackChanged => {
            let info = info_of(model.read(cx).player());
            update(info, cx);
        }
    })
    .detach();
}

fn register_commands(model: Entity<AppModel>, cx: &mut App) {
    // MediaPlayer may call handlers off the main thread; hop to the app through a channel.
    let (tx, mut commands) = mpsc::unbounded();
    let center = unsafe { MPRemoteCommandCenter::sharedCommandCenter() };
    let targets: [(Retained<MPRemoteCommand>, Command); 6] = unsafe {
        [
            (center.playCommand(), Command::Play),
            (center.pauseCommand(), Command::Pause),
            (center.stopCommand(), Command::Pause),
            (center.togglePlayPauseCommand(), Command::TogglePlay),
            (center.nextTrackCommand(), Command::Next),
            (center.previousTrackCommand(), Command::Previous),
        ]
    };
    for (remote, command) in targets {
        let tx = tx.clone();
        let handler = RcBlock::new(move |_: NonNull<MPRemoteCommandEvent>| {
            let _ = tx.unbounded_send(command);
            MPRemoteCommandHandlerStatus::Success
        });
        // The command keeps the target alive for the life of the app.
        unsafe {
            remote.setEnabled(true);
            remote.addTargetWithHandler(&handler);
        }
    }

    cx.spawn(async move |cx| {
        while let Some(command) = commands.next().await {
            log::debug!("now playing: {command:?}");
            model.update(cx, |m, cx| match command {
                Command::Play => m.play(cx),
                Command::Pause => m.pause(cx),
                Command::TogglePlay => m.toggle_play(cx),
                Command::Next => m.next(cx),
                Command::Previous => m.previous(cx),
            });
        }
    })
    .detach();
}

fn update(info: Option<Info>, cx: &mut App) {
    let now_playing = cx.global_mut::<NowPlaying>();
    let url = info.as_ref().and_then(|i| i.image_url.clone());
    now_playing.info = info;
    if now_playing.artwork_url != url {
        now_playing.artwork_url = url.clone();
        now_playing.artwork = None;
        if let Some(url) = url {
            fetch_artwork(url, cx);
        }
    }
    publish(cx.global::<NowPlaying>());
}

fn fetch_artwork(url: String, cx: &mut App) {
    cx.spawn(async move |cx| {
        let fetched = {
            let url = url.clone();
            blocking(move || download(&url)).await
        };
        let bytes = match fetched {
            Ok(bytes) => bytes,
            Err(err) => {
                log::warn!("artwork download failed: {err}");
                return;
            }
        };
        cx.update(|cx| {
            let now_playing = cx.global_mut::<NowPlaying>();
            if now_playing.artwork_url.as_deref() != Some(&url) {
                return;
            }
            let Some(artwork) = artwork(&bytes) else {
                log::warn!("artwork is not an image");
                return;
            };
            now_playing.artwork = Some(artwork);
            publish(now_playing);
        });
    })
    .detach();
}

fn download(url: &str) -> Result<Vec<u8>, ureq::Error> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(ARTWORK_TIMEOUT))
        .build()
        .new_agent();
    agent
        .get(url)
        .call()?
        .body_mut()
        .with_config()
        .limit(ARTWORK_LIMIT)
        .read_to_vec()
}

fn artwork(bytes: &[u8]) -> Option<Retained<MPMediaItemArtwork>> {
    let data = NSData::with_bytes(bytes);
    let image = NSImage::initWithData(NSImage::alloc(), &data)?;
    let size = image.size();
    let handler = RcBlock::new(move |_: CGSize| NonNull::from(&*image));
    Some(unsafe {
        MPMediaItemArtwork::initWithBoundsSize_requestHandler(
            MPMediaItemArtwork::alloc(),
            size,
            &handler,
        )
    })
}

fn publish(now_playing: &NowPlaying) {
    debug_assert!(MainThreadMarker::new().is_some());
    let center = unsafe { MPNowPlayingInfoCenter::defaultCenter() };
    let Some(info) = &now_playing.info else {
        unsafe {
            center.setNowPlayingInfo(None);
            center.setPlaybackState(MPNowPlayingPlaybackState::Stopped);
        }
        return;
    };

    let mut keys: Vec<&NSString> = Vec::new();
    let mut values: Vec<Retained<AnyObject>> = Vec::new();
    let mut put = |key: &'static NSString, value: Retained<AnyObject>| {
        keys.push(key);
        values.push(value);
    };
    unsafe {
        put(
            MPMediaItemPropertyTitle,
            NSString::from_str(&info.title).into(),
        );
        put(
            MPMediaItemPropertyArtist,
            NSString::from_str(&info.artist).into(),
        );
        put(
            MPMediaItemPropertyAlbumTitle,
            NSString::from_str(&info.album).into(),
        );
        put(
            MPMediaItemPropertyPlaybackDuration,
            NSNumber::new_f64(info.duration_s).into(),
        );
        put(
            MPNowPlayingInfoPropertyElapsedPlaybackTime,
            NSNumber::new_f64(info.elapsed_s).into(),
        );
        put(
            MPNowPlayingInfoPropertyPlaybackRate,
            NSNumber::new_f64(if info.playing { 1.0 } else { 0.0 }).into(),
        );
        if let Some(artwork) = &now_playing.artwork {
            put(MPMediaItemPropertyArtwork, artwork.clone().into());
        }
    }
    let dictionary = NSDictionary::from_retained_objects(&keys, &values);
    unsafe {
        center.setNowPlayingInfo(Some(&dictionary));
        center.setPlaybackState(if info.playing {
            MPNowPlayingPlaybackState::Playing
        } else {
            MPNowPlayingPlaybackState::Paused
        });
    }
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
    fn carries_the_track_and_times() {
        assert_eq!(
            info_of(&playing()),
            Some(Info {
                title: "Song".into(),
                artist: "A, B".into(),
                album: "Album".into(),
                image_url: Some("https://i.scdn.co/image/x".into()),
                duration_s: 200.0,
                elapsed_s: 61.5,
                playing: true,
            })
        );
    }

    #[test]
    fn paused_or_disconnected_is_not_playing() {
        let paused = PlayerState {
            paused: true,
            ..playing()
        };
        assert!(!info_of(&paused).unwrap().playing);
        let disconnected = PlayerState {
            connected: false,
            ..playing()
        };
        assert!(!info_of(&disconnected).unwrap().playing);
    }

    #[test]
    fn elapsed_never_passes_the_duration() {
        let over = PlayerState {
            position_ms: 250_000,
            ..playing()
        };
        assert_eq!(info_of(&over).unwrap().elapsed_s, 200.0);
    }

    #[test]
    fn clears_without_a_track_or_while_playing_elsewhere() {
        let empty = PlayerState {
            track: None,
            ..playing()
        };
        assert_eq!(info_of(&empty), None);
        let elsewhere = PlayerState {
            elsewhere: Some("Phone".into()),
            ..playing()
        };
        assert_eq!(info_of(&elsewhere), None);
    }
}

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::api::{Error, Method, RequestOpts, WebApi};
use crate::types::{
    Artist, BridgeError, BridgeErrorCode, ElsewhereTrack, PlayingElsewhere, Source,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resume {
    pub track_uri: Option<String>,
    pub position_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlayBody {
    pub context_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<Offset>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Offset {
    pub uri: String,
}

pub fn build_play_body(source: &Source, user_id: &str, resume: Option<&Resume>) -> PlayBody {
    // The collection URI is undocumented; docs/spec/v1.md "Sources" records the spike that shows it working.
    let context_uri = match source {
        Source::Liked => format!("spotify:user:{user_id}:collection"),
        Source::Playlist { id, .. } => format!("spotify:playlist:{id}"),
    };
    let mut body = PlayBody {
        context_uri,
        offset: None,
        position_ms: None,
    };
    if let Some(Resume {
        track_uri: Some(uri),
        position_ms,
    }) = resume
    {
        body.offset = Some(Offset { uri: uri.clone() });
        body.position_ms = Some(*position_ms);
    }
    body
}

#[derive(Debug, Clone, Copy)]
pub struct StartSource<'a> {
    pub source: &'a Source,
    pub user_id: &'a str,
    pub device_id: &'a str,
    pub resume: Option<&'a Resume>,
}

pub fn start_source(api: &impl WebApi, p: StartSource<'_>) -> Result<(), Error> {
    let body = build_play_body(p.source, p.user_id, p.resume);
    let play = |b: &PlayBody| {
        let opts = RequestOpts::default()
            .query("device_id", p.device_id)
            .body(json!(b));
        api.request(Method::Put, "/me/player/play", opts)
    };
    match play(&body) {
        Ok(_) => {}
        Err(Error::Api(e)) => {
            if body.offset.is_none() || !(400..500).contains(&e.status) {
                return Err(play_failed(e.status));
            }
            let bare = PlayBody {
                context_uri: body.context_uri.clone(),
                offset: None,
                position_ms: None,
            };
            play(&bare).map_err(|e2| match e2 {
                Error::Api(e2) => play_failed(e2.status),
                other => other,
            })?;
        }
        Err(other) => return Err(other),
    }

    let set = |path: &str, state: &str| {
        let opts = RequestOpts::default()
            .query("state", state)
            .query("device_id", p.device_id);
        if let Err(e) = api.request(Method::Put, path, opts) {
            log::warn!("{path} failed {e}");
        }
    };
    set("/me/player/shuffle", "false");
    set("/me/player/repeat", "context");
    Ok(())
}

fn play_failed(status: u16) -> Error {
    Error::Bridge(BridgeError::with_status(
        BridgeErrorCode::PlayFailed,
        status,
    ))
}

pub fn transfer_here(api: &impl WebApi, device_id: &str) -> Result<(), Error> {
    let opts = RequestOpts::default().body(json!({ "device_ids": [device_id], "play": true }));
    api.request(Method::Put, "/me/player", opts)?;
    Ok(())
}

#[derive(Deserialize)]
struct PlayerState {
    device: Option<Device>,
    item: Option<Item>,
}

#[derive(Deserialize)]
struct Device {
    id: Option<String>,
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct Item {
    uri: String,
    name: String,
    #[serde(default)]
    artists: Option<Vec<Artist>>,
    #[serde(default)]
    album: Option<Album>,
}

#[derive(Deserialize)]
struct Album {
    name: Option<String>,
    images: Option<Vec<Image>>,
}

#[derive(Deserialize)]
struct Image {
    url: String,
    width: Option<u32>,
}

pub fn get_playing_elsewhere(
    api: &impl WebApi,
    own_device_id: Option<&str>,
) -> Result<Option<PlayingElsewhere>, Error> {
    let res = api.request(Method::Get, "/me/player", RequestOpts::default())?;
    let Some(json) = res.json else {
        return Ok(None);
    };
    let state: PlayerState = serde_json::from_value(json).map_err(Error::Decode)?;
    let Some(device) = state.device else {
        return Ok(None);
    };
    if device.id.as_deref() == own_device_id {
        return Ok(None);
    }
    let track = state.item.map(|item| {
        let album = item.album.unwrap_or(Album {
            name: None,
            images: None,
        });
        ElsewhereTrack {
            uri: item.uri,
            name: item.name,
            artists: item.artists.unwrap_or_default(),
            album: album.name.unwrap_or_default(),
            image_url: largest(album.images.unwrap_or_default()),
        }
    });
    Ok(Some(PlayingElsewhere {
        device_name: device.name,
        track,
    }))
}

fn largest(images: Vec<Image>) -> Option<String> {
    let mut best: Option<Image> = None;
    for img in images {
        if best
            .as_ref()
            .is_none_or(|b| img.width.unwrap_or(0) > b.width.unwrap_or(0))
        {
            best = Some(img);
        }
    }
    best.map(|b| b.url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ApiResponse;
    use crate::fake_api::{FakeApi, api_error, ok, query};
    use crate::test_log;
    use serde_json::{Value, json};

    fn playlist() -> Source {
        Source::Playlist {
            id: "abc".into(),
            uri: "spotify:playlist:abc".into(),
            name: "P".into(),
            image_url: None,
            pasted: false,
        }
    }

    fn resume(track: Option<&str>, position_ms: u64) -> Resume {
        Resume {
            track_uri: track.map(str::to_owned),
            position_ms,
        }
    }

    fn start<'a>(source: &'a Source, resume: Option<&'a Resume>) -> StartSource<'a> {
        StartSource {
            source,
            user_id: "me",
            device_id: "dev",
            resume,
        }
    }

    fn body_json(b: &PlayBody) -> Value {
        serde_json::to_value(b).unwrap()
    }

    #[test]
    fn plays_a_playlist_by_uri() {
        assert_eq!(
            body_json(&build_play_body(&playlist(), "me", None)),
            json!({ "context_uri": "spotify:playlist:abc" })
        );
    }

    #[test]
    fn plays_liked_songs_through_the_collection_uri() {
        assert_eq!(
            body_json(&build_play_body(&Source::Liked, "me", None)),
            json!({ "context_uri": "spotify:user:me:collection" })
        );
    }

    #[test]
    fn adds_offset_and_position_when_resuming_at_a_track() {
        let r = resume(Some("spotify:track:t"), 1234);
        assert_eq!(
            body_json(&build_play_body(&Source::Liked, "me", Some(&r))),
            json!({
                "context_uri": "spotify:user:me:collection",
                "offset": { "uri": "spotify:track:t" },
                "position_ms": 1234,
            })
        );
    }

    #[test]
    fn ignores_a_resume_point_without_a_track() {
        let r = resume(None, 1234);
        assert_eq!(
            body_json(&build_play_body(&playlist(), "me", Some(&r))),
            json!({ "context_uri": "spotify:playlist:abc" })
        );
    }

    #[test]
    fn plays_then_sets_shuffle_off_then_repeat_context_all_on_the_device() {
        let api = FakeApi::ok();
        start_source(&api, start(&playlist(), None)).unwrap();
        let calls = api.calls();
        let summary: Vec<_> = calls
            .iter()
            .map(|c| (c.method, c.path.as_str(), c.query.clone(), c.body.clone()))
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    Method::Put,
                    "/me/player/play",
                    query(&[("device_id", "dev")]),
                    Some(json!({ "context_uri": "spotify:playlist:abc" }))
                ),
                (
                    Method::Put,
                    "/me/player/shuffle",
                    query(&[("state", "false"), ("device_id", "dev")]),
                    None
                ),
                (
                    Method::Put,
                    "/me/player/repeat",
                    query(&[("state", "context"), ("device_id", "dev")]),
                    None
                ),
            ]
        );
    }

    #[test]
    fn retries_without_the_offset_when_the_play_call_fails_with_4xx() {
        let api = FakeApi::new(|c| {
            let has_offset = c.body.as_ref().is_some_and(|b| b.get("offset").is_some());
            if c.path == "/me/player/play" && has_offset {
                api_error(c, 404)
            } else {
                Ok(ApiResponse {
                    status: 200,
                    json: None,
                })
            }
        });
        let r = resume(Some("spotify:track:t"), 5);
        start_source(&api, start(&playlist(), Some(&r))).unwrap();
        let calls = api.calls();
        let paths: Vec<_> = calls.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "/me/player/play",
                "/me/player/play",
                "/me/player/shuffle",
                "/me/player/repeat"
            ]
        );
        assert_eq!(
            calls[0].body,
            Some(
                json!({ "context_uri": "spotify:playlist:abc", "offset": { "uri": "spotify:track:t" }, "position_ms": 5 })
            )
        );
        assert_eq!(
            calls[1].body,
            Some(json!({ "context_uri": "spotify:playlist:abc" }))
        );
    }

    fn play_fails_with(status: u16) -> FakeApi {
        FakeApi::new(move |c| {
            if c.path == "/me/player/play" {
                api_error(c, status)
            } else {
                Ok(ApiResponse {
                    status: 200,
                    json: None,
                })
            }
        })
    }

    fn assert_play_failed(err: Error, status: u16) {
        match err {
            Error::Bridge(b) => assert_eq!(
                b,
                BridgeError::with_status(BridgeErrorCode::PlayFailed, status)
            ),
            other => panic!("expected play-failed, got {other:?}"),
        }
    }

    #[test]
    fn fails_play_failed_with_the_status_when_the_retry_also_fails() {
        let api = play_fails_with(404);
        let r = resume(Some("spotify:track:t"), 5);
        assert_play_failed(
            start_source(&api, start(&playlist(), Some(&r))).unwrap_err(),
            404,
        );
        assert_eq!(api.calls().len(), 2);
    }

    #[test]
    fn does_not_retry_a_4xx_when_there_was_no_offset_to_drop() {
        let api = play_fails_with(403);
        assert_play_failed(
            start_source(&api, start(&playlist(), None)).unwrap_err(),
            403,
        );
        assert_eq!(api.calls().len(), 1);
    }

    #[test]
    fn does_not_retry_a_5xx_without_the_offset() {
        let api = play_fails_with(502);
        let r = resume(Some("spotify:track:t"), 5);
        assert_play_failed(
            start_source(&api, start(&playlist(), Some(&r))).unwrap_err(),
            502,
        );
        assert_eq!(api.calls().len(), 1);
    }

    #[test]
    fn logs_but_does_not_fail_when_shuffle_or_repeat_fail() {
        let api = FakeApi::new(|c| {
            if c.path == "/me/player/play" {
                Ok(ApiResponse {
                    status: 200,
                    json: None,
                })
            } else {
                api_error(c, 500)
            }
        });
        let logs = test_log::capture(|| start_source(&api, start(&Source::Liked, None)).unwrap());
        let paths: Vec<_> = api.calls().iter().map(|c| c.path.clone()).collect();
        assert_eq!(
            paths,
            ["/me/player/play", "/me/player/shuffle", "/me/player/repeat"]
        );
        assert_eq!(logs.len(), 2);
    }

    #[test]
    fn transfer_moves_playback_to_the_device_and_starts_it() {
        let api = FakeApi::ok();
        transfer_here(&api, "dev").unwrap();
        let calls = api.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            (calls[0].method, calls[0].path.as_str()),
            (Method::Put, "/me/player")
        );
        assert!(calls[0].query.is_empty());
        assert_eq!(
            calls[0].body,
            Some(json!({ "device_ids": ["dev"], "play": true }))
        );
    }

    fn state(device_id: &str, item: Option<Value>) -> Value {
        let mut s = json!({ "device": { "id": device_id, "name": "Kitchen" } });
        if let Some(item) = item {
            s["item"] = item;
        }
        s
    }

    fn track() -> Value {
        json!({
            "uri": "spotify:track:t1",
            "name": "Song",
            "artists": [{ "name": "A", "uri": "spotify:artist:a" }, { "name": "B", "uri": "spotify:artist:b" }],
            "album": {
                "name": "Album",
                "images": [
                    { "url": "small.jpg", "width": 64 },
                    { "url": "big.jpg", "width": 640 },
                    { "url": "mid.jpg", "width": 300 }
                ]
            }
        })
    }

    #[test]
    fn elsewhere_is_none_on_204() {
        let api = FakeApi::new(|_| {
            Ok(ApiResponse {
                status: 204,
                json: None,
            })
        });
        assert_eq!(get_playing_elsewhere(&api, Some("own")).unwrap(), None);
    }

    #[test]
    fn elsewhere_is_none_when_the_active_device_is_our_own() {
        let api = FakeApi::new(|_| ok(state("own", Some(track()))));
        assert_eq!(get_playing_elsewhere(&api, Some("own")).unwrap(), None);
    }

    #[test]
    fn elsewhere_gives_the_device_and_track_when_another_device_is_active() {
        let api = FakeApi::new(|_| ok(state("other", Some(track()))));
        let got = get_playing_elsewhere(&api, Some("own")).unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(got).unwrap(),
            json!({
                "deviceName": "Kitchen",
                "track": {
                    "uri": "spotify:track:t1",
                    "name": "Song",
                    "artists": [{ "name": "A", "uri": "spotify:artist:a" }, { "name": "B", "uri": "spotify:artist:b" }],
                    "album": "Album",
                    "imageUrl": "big.jpg",
                },
            })
        );
    }

    #[test]
    fn elsewhere_gives_just_the_device_name_when_there_is_no_item() {
        let api = FakeApi::new(|_| ok(state("other", None)));
        assert_eq!(
            get_playing_elsewhere(&api, None).unwrap(),
            Some(PlayingElsewhere {
                device_name: "Kitchen".into(),
                track: None
            })
        );
    }
}

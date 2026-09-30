use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde::Deserialize;

use crate::api::{Error, Method, RequestOpts, WebApi};
use crate::types::{BridgeError, BridgeErrorCode, Source};

const ID_LEN: usize = 22;

/// The playlist id in a pasted link, or `None` if the text is not one of the forms
/// docs/spec/v1.md "Sources" accepts.
pub fn parse_pasted_link(text: &str) -> Option<&str> {
    let trimmed = text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    let web = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"));
    if let Some(rest) = web.and_then(|r| r.strip_prefix("open.spotify.com/playlist/")) {
        return id_then(rest, &['/', '?', '#']);
    }
    if let Some(rest) = trimmed.strip_prefix("spotify:playlist:") {
        return id_then(rest, &['?', '#']);
    }
    id_then(trimmed, &[])
}

/// A 22-character base-62 id, followed by nothing or by one of `tails` and anything up to the end
/// of the line. The line limit is the TypeScript regex's `.`, which does not match line breaks.
fn id_then<'a>(s: &'a str, tails: &[char]) -> Option<&'a str> {
    let id = s.get(..ID_LEN)?;
    if !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let rest = &s[ID_LEN..];
    let ok = match rest.chars().next() {
        None => true,
        Some(c) => tails.contains(&c) && !rest.contains(['\n', '\r', '\u{2028}', '\u{2029}']),
    };
    ok.then_some(id)
}

#[derive(Deserialize)]
struct PlaylistJson {
    #[serde(default)]
    id: String,
    #[serde(default)]
    uri: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    images: Option<Vec<ImageJson>>,
}

#[derive(Deserialize)]
struct ImageJson {
    url: String,
}

#[derive(Deserialize)]
struct Page {
    #[serde(default)]
    items: Vec<Option<PlaylistJson>>,
    next: Option<String>,
}

fn first_image(images: Option<Vec<ImageJson>>) -> Option<String> {
    images.and_then(|i| i.into_iter().next()).map(|i| i.url)
}

pub struct SourceCatalog<A> {
    api: A,
    cache: Mutex<Option<Arc<Vec<Source>>>>,
    // Held for the length of a fetch. Together with `fetches` it lets a caller that arrives
    // mid-fetch take that fetch's result instead of starting another.
    fetching: Mutex<()>,
    fetches: AtomicU64,
}

impl<A: WebApi> SourceCatalog<A> {
    pub fn new(api: A) -> Self {
        Self {
            api,
            cache: Mutex::new(None),
            fetching: Mutex::new(()),
            fetches: AtomicU64::new(0),
        }
    }

    /// The last fetched list, for callers that must not wait (mapping a context uri to its Source).
    pub fn current(&self) -> Option<Arc<Vec<Source>>> {
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Always fetches; the Picker renders its own cached copy first and swaps in this result.
    pub fn list(&self) -> Result<Arc<Vec<Source>>, Error> {
        self.refresh()
    }

    pub fn refresh(&self) -> Result<Arc<Vec<Source>>, Error> {
        let seen = self.fetches.load(Ordering::SeqCst);
        let _guard = self.fetching.lock().unwrap_or_else(PoisonError::into_inner);
        if self.fetches.load(Ordering::SeqCst) != seen
            && let Some(done) = self.current()
        {
            return Ok(done);
        }
        let sources = Arc::new(self.fetch_all()?);
        *self.cache.lock().unwrap_or_else(PoisonError::into_inner) = Some(sources.clone());
        self.fetches.fetch_add(1, Ordering::SeqCst);
        Ok(sources)
    }

    fn fetch_all(&self) -> Result<Vec<Source>, Error> {
        let mut sources = vec![Source::Liked];
        let mut res = self.api.request(
            Method::Get,
            "/me/playlists",
            RequestOpts::default().query("limit", "50"),
        )?;
        loop {
            let page: Option<Page> = res
                .json
                .map(serde_json::from_value)
                .transpose()
                .map_err(Error::Decode)?;
            let Some(page) = page else { break };
            for p in page.items.into_iter().flatten() {
                sources.push(Source::Playlist {
                    id: p.id,
                    uri: p.uri,
                    name: p.name,
                    image_url: first_image(p.images),
                    pasted: false,
                });
            }
            let Some(next) = page.next.filter(|n| !n.is_empty()) else {
                break;
            };
            res = self
                .api
                .request(Method::Get, &next, RequestOpts::default())?;
        }
        Ok(sources)
    }
}

pub fn resolve_pasted_link(api: &impl WebApi, text: &str) -> Result<Source, Error> {
    let Some(id) = parse_pasted_link(text) else {
        return Err(BridgeError::new(BridgeErrorCode::BadLink).into());
    };
    let opts = RequestOpts::default().query("fields", "name,uri,images,owner.id");
    let res = match api.request(Method::Get, &format!("/playlists/{id}"), opts) {
        Ok(res) => res,
        Err(Error::Api(e)) if e.status == 404 => {
            return Err(BridgeError::new(BridgeErrorCode::NotFound).into());
        }
        Err(Error::Api(e)) if e.status == 403 => {
            return Err(BridgeError::new(BridgeErrorCode::Forbidden).into());
        }
        Err(e) => return Err(e),
    };
    let Some(json) = res.json else {
        return Err(BridgeError::new(BridgeErrorCode::NotFound).into());
    };
    let p: PlaylistJson = serde_json::from_value(json).map_err(Error::Decode)?;
    Ok(Source::Playlist {
        id: id.to_owned(),
        uri: p.uri,
        name: p.name,
        image_url: first_image(p.images),
        pasted: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ApiResponse;
    use crate::fake_api::{FakeApi, api_error, ok, query};
    use serde_json::{Value, json};
    use std::sync::atomic::AtomicU32;
    use std::sync::mpsc;
    use std::thread;

    const ID: &str = "37i9dQZF1DXcBWIGoYBM5M";

    #[test]
    fn accepts_every_documented_form() {
        let cases = [
            format!("https://open.spotify.com/playlist/{ID}"),
            format!("https://open.spotify.com/playlist/{ID}/"),
            format!("https://open.spotify.com/playlist/{ID}?si=abc123&nd=1"),
            format!("  https://open.spotify.com/playlist/{ID}?si=abc123 \n"),
            format!("http://open.spotify.com/playlist/{ID}"),
            format!("spotify:playlist:{ID}"),
            format!("spotify:playlist:{ID}?si=x"),
            ID.to_owned(),
            format!("  {ID}  "),
        ];
        for text in &cases {
            assert_eq!(parse_pasted_link(text), Some(ID), "accepts {text:?}");
        }
    }

    #[test]
    fn rejects_everything_else() {
        let cases = [
            format!("https://open.spotify.com/track/{ID}"),
            format!("https://open.spotify.com/album/{ID}?si=x"),
            format!("https://open.spotify.com/artist/{ID}"),
            format!("spotify:track:{ID}"),
            format!("spotify:album:{ID}"),
            format!("https://example.com/playlist/{ID}"),
            "https://open.spotify.com/playlist/".to_owned(),
            "not a link at all".to_owned(),
            ID[..21].to_owned(),
            format!("{ID}x"),
            format!("{}-", &ID[..21]),
            String::new(),
            // Not in the TypeScript cases: the regex's `.` stops at a line break.
            format!("https://open.spotify.com/playlist/{ID}?si=a\nb"),
            // A multi-byte character where the id should end must not panic on slicing.
            format!("{}é", &ID[..21]),
        ];
        for text in &cases {
            assert_eq!(parse_pasted_link(text), None, "rejects {text:?}");
        }
    }

    fn playlist(n: u32, images: &[&str]) -> Value {
        let images: Vec<Value> = images.iter().map(|u| json!({ "url": u })).collect();
        json!({ "id": format!("id{n}"), "uri": format!("spotify:playlist:id{n}"), "name": format!("Playlist {n}"), "images": images })
    }

    fn source(n: u32, image: Option<&str>) -> Source {
        Source::Playlist {
            id: format!("id{n}"),
            uri: format!("spotify:playlist:id{n}"),
            name: format!("Playlist {n}"),
            image_url: image.map(str::to_owned),
            pasted: false,
        }
    }

    const NEXT: &str = "https://api.spotify.com/v1/me/playlists?offset=50&limit=50";

    #[test]
    fn lists_liked_first_then_playlists_across_every_page() {
        let api = FakeApi::new(|c| match c.path.as_str() {
            "/me/playlists" => {
                ok(json!({ "items": [playlist(1, &["a.jpg", "b.jpg"])], "next": NEXT }))
            }
            NEXT => ok(json!({ "items": [playlist(2, &[])], "next": null })),
            other => panic!("unexpected {other}"),
        });
        let catalog = SourceCatalog::new(&api);
        let sources = catalog.list().unwrap();
        assert_eq!(
            *sources,
            vec![Source::Liked, source(1, Some("a.jpg")), source(2, None)]
        );
        let calls = api.calls();
        assert_eq!(
            (calls[0].method, calls[0].path.as_str()),
            (Method::Get, "/me/playlists")
        );
        assert_eq!(calls[0].query, query(&[("limit", "50")]));
        assert_eq!(calls.len(), 2);
    }

    fn counting_api() -> FakeApi {
        let n = AtomicU32::new(0);
        FakeApi::new(move |_| {
            let i = n.fetch_add(1, Ordering::SeqCst) + 1;
            ok(json!({ "items": [playlist(i, &[])], "next": null }))
        })
    }

    #[test]
    fn fetches_on_every_list_and_keeps_the_last_result_as_current() {
        let api = counting_api();
        let catalog = SourceCatalog::new(&api);
        assert_eq!(catalog.current(), None);
        let first = catalog.list().unwrap();
        assert_eq!(first[1], source(1, None));
        let second = catalog.list().unwrap();
        assert_eq!(second[1], source(2, None));
        assert_eq!(api.calls().len(), 2);
        assert!(Arc::ptr_eq(&catalog.current().unwrap(), &second));
    }

    #[test]
    fn refresh_always_fetches_and_replaces_the_cache() {
        let api = counting_api();
        let catalog = SourceCatalog::new(&api);
        catalog.list().unwrap();
        let refreshed = catalog.refresh().unwrap();
        assert_eq!(refreshed[1], source(2, None));
    }

    #[test]
    fn a_caller_arriving_mid_fetch_shares_that_fetch() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let first = AtomicU32::new(0);
        let api = FakeApi::new(move |_| {
            // Only the first fetch blocks, so a second one fails the assertion instead of hanging.
            if first.fetch_add(1, Ordering::SeqCst) == 0 {
                entered_tx.send(()).unwrap();
                release_rx.lock().unwrap().recv().unwrap();
            }
            ok(json!({ "items": [], "next": null }))
        });
        let catalog = SourceCatalog::new(&api);
        thread::scope(|s| {
            let a = s.spawn(|| catalog.refresh().unwrap());
            entered_rx.recv().unwrap();
            let b = s.spawn(|| catalog.refresh().unwrap());
            // Give b time to start waiting on the first fetch.
            thread::sleep(std::time::Duration::from_millis(50));
            release_tx.send(()).unwrap();
            let (a, b) = (a.join().unwrap(), b.join().unwrap());
            assert!(Arc::ptr_eq(&a, &b));
        });
        assert_eq!(api.calls().len(), 1);
    }

    #[test]
    fn rejects_unparseable_text_with_bad_link_before_calling_the_api() {
        let api = FakeApi::new(|_| ok(json!({})));
        let err = resolve_pasted_link(&api, "nope").unwrap_err();
        assert!(matches!(
            err,
            Error::Bridge(BridgeError {
                code: BridgeErrorCode::BadLink,
                ..
            })
        ));
        assert!(api.calls().is_empty());
    }

    #[test]
    fn fetches_the_playlist_and_returns_a_pasted_source() {
        let api = FakeApi::new(|_| {
            ok(
                json!({ "name": "Mix", "uri": format!("spotify:playlist:{ID}"), "images": [{ "url": "x.jpg" }], "owner": { "id": "someone" } }),
            )
        });
        let src = resolve_pasted_link(
            &api,
            &format!("https://open.spotify.com/playlist/{ID}?si=1"),
        )
        .unwrap();
        assert_eq!(
            src,
            Source::Playlist {
                id: ID.into(),
                uri: format!("spotify:playlist:{ID}"),
                name: "Mix".into(),
                image_url: Some("x.jpg".into()),
                pasted: true,
            }
        );
        let calls = api.calls();
        assert_eq!(
            (calls[0].method, calls[0].path.clone()),
            (Method::Get, format!("/playlists/{ID}"))
        );
        assert_eq!(
            calls[0].query,
            query(&[("fields", "name,uri,images,owner.id")])
        );
    }

    fn code_for(status: u16) -> Option<BridgeErrorCode> {
        let api = FakeApi::new(move |c| api_error(c, status));
        match resolve_pasted_link(&api, ID).unwrap_err() {
            Error::Bridge(b) => Some(b.code),
            _ => None,
        }
    }

    #[test]
    fn maps_404_to_not_found_and_403_to_forbidden() {
        assert_eq!(code_for(404), Some(BridgeErrorCode::NotFound));
        assert_eq!(code_for(403), Some(BridgeErrorCode::Forbidden));
    }

    #[test]
    fn passes_other_api_errors_through() {
        let api = FakeApi::new(|c| api_error(c, 500));
        assert_eq!(
            resolve_pasted_link(&api, ID).unwrap_err().api_status(),
            Some(500)
        );
    }

    #[test]
    fn maps_a_2xx_without_a_body_to_not_found() {
        let api = FakeApi::new(|_| {
            Ok(ApiResponse {
                status: 204,
                json: None,
            })
        });
        let err = resolve_pasted_link(&api, ID).unwrap_err();
        assert!(matches!(
            err,
            Error::Bridge(BridgeError {
                code: BridgeErrorCode::NotFound,
                ..
            })
        ));
    }
}

//! Display helpers the Dropdown needs: times, artwork and open.spotify.com links.

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Image {
    pub url: String,
    #[serde(default)]
    pub width: Option<f64>,
}

/// `m:ss`, with minutes past 59 left as they are.
pub fn format_time(ms: u64) -> String {
    let total = ms / 1000;
    format!("{}:{:02}", total / 60, total % 60)
}

pub fn largest_image(images: &[Image]) -> Option<&Image> {
    let mut best: Option<&Image> = None;
    for image in images {
        if best.is_none_or(|b| image.width.unwrap_or(0.0) > b.width.unwrap_or(0.0)) {
            best = Some(image);
        }
    }
    best
}

/// `spotify:<kind>:<id>` to its open.spotify.com page, or `None` for anything else.
pub fn spotify_url(uri: &str, kind: &str) -> Option<String> {
    let mut parts = uri.split(':');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("spotify"), Some(k), Some(id), None) if k == kind && !id.is_empty() => {
            Some(format!("https://open.spotify.com/{kind}/{id}"))
        }
        _ => None,
    }
}

#[allow(dead_code, reason = "the Dropdown UI (#28) links the title")]
pub fn track_url(uri: &str) -> Option<String> {
    spotify_url(uri, "track")
}

#[allow(dead_code, reason = "the Dropdown UI (#28) links the first artist")]
pub fn artist_url(uri: &str) -> Option<String> {
    spotify_url(uri, "artist")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_m_ss() {
        assert_eq!(format_time(0), "0:00");
        assert_eq!(format_time(999), "0:00");
        assert_eq!(format_time(1000), "0:01");
        assert_eq!(format_time(65_000), "1:05");
        assert_eq!(format_time(598_000), "9:58");
        assert_eq!(format_time(3_725_000), "62:05");
    }

    fn image(url: &str, width: Option<f64>) -> Image {
        Image {
            url: url.into(),
            width,
        }
    }

    #[test]
    fn picks_the_widest_image() {
        let images = [
            image("a", Some(64.0)),
            image("b", Some(640.0)),
            image("c", Some(300.0)),
        ];
        assert_eq!(largest_image(&images).unwrap().url, "b");
    }

    #[test]
    fn handles_no_images_and_missing_sizes() {
        assert_eq!(largest_image(&[]), None);
        assert_eq!(largest_image(&[image("a", None)]).unwrap().url, "a");
    }

    #[test]
    fn maps_uris_to_open_spotify_com() {
        assert_eq!(
            track_url("spotify:track:4uLU6hMCjMI75M1A2tKUQC").as_deref(),
            Some("https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC")
        );
        assert_eq!(
            artist_url("spotify:artist:abc").as_deref(),
            Some("https://open.spotify.com/artist/abc")
        );
    }

    #[test]
    fn rejects_the_wrong_kind_of_uri() {
        assert_eq!(track_url("spotify:artist:abc"), None);
        assert_eq!(artist_url("spotify:track:abc"), None);
        assert_eq!(track_url(""), None);
        assert_eq!(track_url("spotify:track:"), None);
        assert_eq!(track_url("spotify:track:a:b"), None);
    }
}

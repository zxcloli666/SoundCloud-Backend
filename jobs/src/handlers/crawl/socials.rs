use catalog_sources::{GeniusArtistDetails, MbArtistUrl};
use serde_json::Value;
use sqlx::PgPool;
use tracing::debug;
use uuid::Uuid;

use crate::handlers::catalog_read::PublicCatalogReader;

use super::error::CrawlResult;

#[derive(Debug, Clone)]
pub struct SocialLink {
    pub kind: String,
    pub url: String,
    pub source: String,
}

pub fn from_musicbrainz(url: &MbArtistUrl) -> Option<SocialLink> {
    let trimmed = url.url.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(SocialLink {
        kind: classify(trimmed).unwrap_or_else(|| url.kind.clone()),
        url: trimmed.to_owned(),
        source: "mb".to_owned(),
    })
}

pub fn from_genius(details: &GeniusArtistDetails) -> Vec<SocialLink> {
    let handle = |kind: &str, host: &str, name: Option<&str>| {
        name.filter(|name| !name.is_empty()).map(|name| SocialLink {
            kind: kind.to_owned(),
            url: format!("https://{host}/{name}"),
            source: "genius".to_owned(),
        })
    };
    let mut links: Vec<SocialLink> = [
        handle("instagram", "instagram.com", details.instagram.as_deref()),
        handle("twitter", "twitter.com", details.twitter.as_deref()),
        handle("facebook", "facebook.com", details.facebook.as_deref()),
    ]
    .into_iter()
    .flatten()
    .collect();
    if let Some(url) = details.url.as_deref().filter(|url| !url.is_empty()) {
        links.push(SocialLink {
            kind: "genius".to_owned(),
            url: url.to_owned(),
            source: "genius".to_owned(),
        });
    }
    links
}

pub async fn from_soundcloud_profile(
    reader: &PublicCatalogReader,
    sc_user_id: &str,
) -> Vec<SocialLink> {
    let path = format!("/users/soundcloud:users:{sc_user_id}/web-profiles");
    let value = match reader.get_json(&path).await {
        Ok(value) => value,
        Err(error) => {
            debug!(sc_user_id, %error, "soundcloud web-profiles unavailable");
            return Vec::new();
        }
    };
    parse_web_profiles(&value)
}

pub async fn store(pool: &PgPool, artist_id: Uuid, links: &[SocialLink]) -> CrawlResult {
    for link in links {
        sqlx::query_file!(
            "queries/crawl/upsert_artist_social.sql",
            artist_id,
            &link.kind,
            &link.url,
            &link.source
        )
        .execute(pool)
        .await?;
    }
    Ok(())
}

fn parse_web_profiles(value: &Value) -> Vec<SocialLink> {
    let entries = match value.as_array() {
        Some(entries) => entries,
        None => match value.get("collection").and_then(Value::as_array) {
            Some(entries) => entries,
            None => return Vec::new(),
        },
    };
    entries
        .iter()
        .filter_map(|entry| {
            let url = entry
                .get("url")
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())?;
            let kind = classify(url).unwrap_or_else(|| {
                entry
                    .get("network")
                    .or_else(|| entry.get("service"))
                    .and_then(Value::as_str)
                    .unwrap_or("other")
                    .to_owned()
            });
            Some(SocialLink {
                kind,
                url: url.to_owned(),
                source: "sc".to_owned(),
            })
        })
        .collect()
}

fn classify(raw: &str) -> Option<String> {
    let parsed = url::Url::parse(raw).ok()?;
    let host = parsed.host_str()?.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(host.as_str());
    let kind = match host {
        "instagram.com" => "instagram",
        "twitter.com" | "x.com" | "mobile.twitter.com" => "twitter",
        "facebook.com" | "m.facebook.com" | "fb.com" | "fb.me" => "facebook",
        "youtube.com" | "youtu.be" | "music.youtube.com" => "youtube",
        "soundcloud.com" | "m.soundcloud.com" => "soundcloud",
        "spotify.com" | "open.spotify.com" => "spotify",
        "music.apple.com" | "itunes.apple.com" => "apple_music",
        "bandcamp.com" => "bandcamp",
        "tiktok.com" | "vm.tiktok.com" => "tiktok",
        "discogs.com" => "discogs",
        "last.fm" | "lastfm.com" | "lastfm.de" => "lastfm",
        "genius.com" => "genius",
        "musicbrainz.org" => "musicbrainz",
        "vk.com" => "vk",
        "telegram.me" | "t.me" => "telegram",
        "wikipedia.org" => "wikipedia",
        host if host.ends_with(".bandcamp.com") => "bandcamp",
        host if host.ends_with(".wikipedia.org") => "wikipedia",
        host if host.ends_with(".allmusic.com") || host == "allmusic.com" => "allmusic",
        host if host.ends_with(".bandsintown.com") || host == "bandsintown.com" => "bandsintown",
        _ => return None,
    };
    Some(kind.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_array_of_profiles_is_read() {
        let value = serde_json::json!([
            { "url": "https://instagram.com/artist" },
            { "url": "https://example.com/artist", "network": "personal" }
        ]);

        let links = parse_web_profiles(&value);

        assert_eq!(links.len(), 2);
        assert_eq!(links[0].kind, "instagram");
        assert_eq!(links[1].kind, "personal");
    }

    #[test]
    fn a_collection_envelope_is_read_the_same_way() {
        let value = serde_json::json!({
            "collection": [{ "url": "https://x.com/artist" }]
        });

        let links = parse_web_profiles(&value);

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, "twitter");
    }

    #[test]
    fn subdomains_of_known_hosts_keep_their_kind() {
        assert_eq!(
            classify("https://artist.bandcamp.com/album/one").as_deref(),
            Some("bandcamp")
        );
        assert_eq!(classify("https://unknown.example/artist"), None);
    }
}

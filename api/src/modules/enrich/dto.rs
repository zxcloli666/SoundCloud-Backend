use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use mini_moka::sync::Cache;
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::sc_ids::normalize_sc_track_id;
use crate::error::AppResult;

#[derive(Debug, Clone, Serialize)]
pub struct EnrichmentDto {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    pub upload_kind: String,
    pub availability: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_artist: Option<ArtistDto>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub participants: Vec<ParticipantDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<AlbumDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_year: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtistDto {
    pub id: Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sc_user_id: Option<String>,
    pub source: String,
    pub confidence: f32,
    pub verified: bool,
}

fn artist_verified(source: &str, has_identity_account: bool) -> bool {
    has_identity_account || matches!(source, "isrc" | "mb" | "genius" | "spotify")
}

#[derive(Debug, Clone, Serialize)]
pub struct ParticipantDto {
    pub artist: ArtistDto,
    pub role: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlbumDto {
    pub id: Uuid,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_artist: Option<ArtistDto>,
}

const PARTICIPANTS_TTL: Duration = Duration::from_secs(60);
const PARTICIPANTS_KEPT: u64 = 200_000;
const PARTICIPANTS_CACHE: &str = "track_participants";

static PARTICIPANTS: LazyLock<Cache<Uuid, Arc<Vec<ParticipantDto>>>> = LazyLock::new(|| {
    Cache::builder()
        .max_capacity(PARTICIPANTS_KEPT)
        .time_to_live(PARTICIPANTS_TTL)
        .build()
});

async fn participants_of(
    pg: &PgPool,
    track_ids: &[Uuid],
) -> AppResult<HashMap<Uuid, Arc<Vec<ParticipantDto>>>> {
    let mut found = HashMap::with_capacity(track_ids.len());
    let mut missing = Vec::new();
    for track_id in track_ids {
        match PARTICIPANTS.get(track_id) {
            Some(participants) => {
                crate::metrics::record_cache(PARTICIPANTS_CACHE, true);
                found.insert(*track_id, participants);
            }
            None => {
                crate::metrics::record_cache(PARTICIPANTS_CACHE, false);
                missing.push(*track_id);
            }
        }
    }
    if missing.is_empty() {
        return Ok(found);
    }
    missing.sort_unstable();
    missing.dedup();
    let rows = sqlx::query_file!("queries/enrich/dto/lookup_participants.sql", &missing)
        .fetch_all(pg)
        .await?;
    let mut loaded: HashMap<Uuid, Vec<ParticipantDto>> =
        missing.iter().map(|id| (*id, Vec::new())).collect();
    for p in rows {
        let verified = artist_verified(&p.artist_source, p.artist_identity);
        loaded.entry(p.track_id).or_default().push(ParticipantDto {
            artist: ArtistDto {
                id: p.artist_id,
                name: p.artist_name,
                avatar_url: p.artist_avatar_url,
                sc_user_id: p.artist_sc_user_id,
                source: p.artist_source,
                confidence: p.artist_confidence,
                verified,
            },
            role: p.role,
            confidence: p.ta_confidence,
        });
    }
    for (track_id, participants) in loaded {
        let participants = Arc::new(participants);
        PARTICIPANTS.insert(track_id, participants.clone());
        found.insert(track_id, participants);
    }
    crate::metrics::set_cache_size(
        PARTICIPANTS_CACHE,
        PARTICIPANTS.entry_count(),
        PARTICIPANTS.weighted_size(),
    );
    Ok(found)
}

pub async fn lookup(pg: &PgPool, urns: &[String]) -> AppResult<HashMap<String, EnrichmentDto>> {
    let sc_ids: Vec<String> = urns
        .iter()
        .filter_map(|u| normalize_sc_track_id(u))
        .collect();
    if sc_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = sqlx::query_file!("queries/enrich/dto/lookup_tracks.sql", &sc_ids)
        .fetch_all(pg)
        .await?;

    if rows.is_empty() {
        return Ok(HashMap::new());
    }

    let track_ids: Vec<Uuid> = rows.iter().map(|r| r.track_id).collect();
    let mut by_track = participants_of(pg, &track_ids).await?;

    let mut out: HashMap<String, EnrichmentDto> = HashMap::with_capacity(rows.len());
    for r in rows {
        let primary_artist = match (r.pa_id, r.pa_name) {
            (Some(id), Some(name)) => {
                let source = r.pa_source.unwrap_or_else(|| "heuristic".to_string());
                let verified = artist_verified(&source, r.pa_identity);
                Some(ArtistDto {
                    id,
                    name,
                    avatar_url: r.pa_avatar_url,
                    sc_user_id: r.pa_sc_user_id,
                    source,
                    confidence: r.pa_confidence.unwrap_or(0.0),
                    verified,
                })
            }
            _ => None,
        };
        let album = match (r.al_id, r.al_title, r.al_kind) {
            (Some(id), Some(title), Some(kind)) => Some(AlbumDto {
                id,
                title,
                year: r.al_release_year,
                cover_url: r.al_cover_url,
                kind,
                primary_artist: match (r.aa_id, r.aa_name) {
                    (Some(aid), Some(aname)) => {
                        let source = r.aa_source.unwrap_or_else(|| "heuristic".to_string());
                        let verified = artist_verified(&source, r.aa_identity);
                        Some(ArtistDto {
                            id: aid,
                            name: aname,
                            avatar_url: r.aa_avatar_url,
                            sc_user_id: r.aa_sc_user_id,
                            source,
                            confidence: r.aa_confidence.unwrap_or(0.0),
                            verified,
                        })
                    }
                    _ => None,
                },
            }),
            _ => None,
        };
        let mut participants = by_track
            .remove(&r.track_id)
            .map(|cached| cached.as_ref().clone())
            .unwrap_or_default();
        if let Some(pa) = primary_artist.as_ref() {
            participants.retain(|p| !(p.role == "primary" && p.artist.id == pa.id));
        }
        let (release_year, release_date, release_source) = if let Some(date) = r.it_release_date {
            (
                r.it_release_year
                    .or_else(|| date.format("%Y").to_string().parse::<i16>().ok()),
                Some(date.format("%Y-%m-%d").to_string()),
                Some("sc_upload".to_string()),
            )
        } else if let Some(y) = r.it_release_year {
            (Some(y), None, Some("sc_upload".to_string()))
        } else if let Some(y) = album.as_ref().and_then(|a| a.year) {
            (Some(y), None, Some("album".to_string()))
        } else {
            (None, None, None)
        };
        out.insert(
            r.sc_track_id,
            EnrichmentDto {
                state: r.enrich_state,
                source: r.enrich_source,
                confidence: r.enrich_confidence,
                upload_kind: r.upload_kind,
                availability: "indexed".to_string(),
                primary_artist,
                participants,
                album,
                release_year,
                release_date,
                release_source,
            },
        );
    }
    Ok(out)
}

fn parse_year(s: &str) -> Option<i16> {
    s.get(..4)?
        .parse::<i16>()
        .ok()
        .filter(|y| (1900..=2100).contains(y))
}

fn parse_iso_date(s: &str) -> Option<String> {
    let head = s.get(..10)?;
    let ok = head.bytes().enumerate().all(|(i, b)| match i {
        4 | 7 => b == b'-',
        _ => b.is_ascii_digit(),
    });
    ok.then(|| head.to_string())
}

fn extract_sc_release(track: &Value) -> (Option<i16>, Option<String>) {
    let candidates = ["release_date", "display_date", "created_at"];
    for key in candidates {
        if let Some(s) = track.get(key).and_then(|v| v.as_str()) {
            if let Some(date) = parse_iso_date(s) {
                let year = parse_year(&date);
                return (year, Some(date));
            }
            if let Some(year) = parse_year(s) {
                return (Some(year), None);
            }
        }
    }
    let release_year = track
        .get("release_year")
        .and_then(|v| v.as_i64())
        .filter(|y| (1900..=2100).contains(&(*y as i16)))
        .map(|y| y as i16);
    (release_year, None)
}

pub async fn apply_to_tracks(pg: &PgPool, tracks: &mut [Value]) -> AppResult<()> {
    if tracks.is_empty() {
        return Ok(());
    }
    let urns: Vec<String> = tracks
        .iter()
        .filter_map(|t| t.get("urn").and_then(|v| v.as_str()).map(String::from))
        .collect();
    if urns.is_empty() {
        return Ok(());
    }
    crate::modules::tracks::counters::sync(pg, tracks).await?;
    let map = lookup(pg, &urns).await?;
    if map.is_empty() {
        return Ok(());
    }
    for t in tracks.iter_mut() {
        let Some(urn) = t.get("urn").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(sc_id) = normalize_sc_track_id(urn) else {
            continue;
        };
        let Some(enrichment) = map.get(&sc_id) else {
            continue;
        };
        let mut filled = enrichment.clone();
        if filled.release_year.is_none() {
            let (year, date) = extract_sc_release(t);
            if year.is_some() || date.is_some() {
                filled.release_year = year;
                filled.release_date = date;
                filled.release_source = Some("sc_upload".to_string());
            }
        }
        if let Some(obj) = t.as_object_mut()
            && let Ok(value) = serde_json::to_value(&filled)
        {
            obj.insert("enrichment".into(), value);
        }
    }
    Ok(())
}

pub async fn apply_to_track(pg: &PgPool, track: &mut Value) -> AppResult<()> {
    apply_to_tracks(pg, std::slice::from_mut(track)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn credit(pool: &PgPool, role: &str) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO artists (id, name, normalized_name, source)
             VALUES ($1, 'featured', 'featured', 'test') ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::from_u128(7))
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO track_artists (track_id, artist_id, role, source)
             SELECT id, $1, $2, 'test' FROM tracks WHERE sc_track_id = '4242'",
        )
        .bind(Uuid::from_u128(7))
        .bind(role)
        .execute(pool)
        .await?;
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn participants_are_served_from_the_cache_until_it_expires(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms)
             VALUES ('4242', 'soundcloud:tracks:4242', 'T', 't', 1000)",
        )
        .execute(&pool)
        .await?;
        let urns = vec!["soundcloud:tracks:4242".to_owned()];
        let before = lookup(&pool, &urns).await?;
        assert!(before["4242"].participants.is_empty());

        credit(&pool, "featured").await?;
        let within = lookup(&pool, &urns).await?;
        assert!(within["4242"].participants.is_empty());

        PARTICIPANTS.invalidate_all();
        let after = lookup(&pool, &urns).await?;
        assert_eq!(after["4242"].participants.len(), 1);
        assert_eq!(after["4242"].participants[0].role, "featured");
        Ok(())
    }
}

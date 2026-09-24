use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Semaphore;

use crate::common::admin::AdminAuth;
use crate::error::AppResult;
use crate::modules::auth::TokenKind;
use crate::state::AppState;

const MAX_REFS: usize = 300;
const MAX_LIVE_USER_LOOKUPS: usize = 40;
const LIVE_LOOKUP_CONCURRENCY: usize = 8;

#[derive(Deserialize)]
pub struct HydrateReq {
    pub refs: Vec<String>,
}

#[derive(Serialize, Clone)]
pub struct EntityCard {
    pub kind: &'static str,
    pub id: String,
    pub urn: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permalink_url: Option<String>,
    pub verified: bool,
}

#[derive(sqlx::FromRow)]
struct UserCardRow {
    sc_user_id: String,
    urn: String,
    username: String,
    permalink: Option<String>,
    avatar_url: Option<String>,
    country: Option<String>,
    city: Option<String>,
    verified: bool,
    followers_count: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct TrackCardRow {
    sc_track_id: String,
    urn: String,
    title: String,
    artwork_url: Option<String>,
    permalink_url: Option<String>,
    uploader_username: Option<String>,
    uploader_avatar_url: Option<String>,
}

#[derive(sqlx::FromRow)]
struct PlaylistCardRow {
    sc_playlist_id: String,
    title: String,
    artwork_url: Option<String>,
    permalink_url: Option<String>,
}

fn parse_urn(s: &str) -> Option<(&str, &str)> {
    let rest = s.strip_prefix("soundcloud:")?;
    let (coll, id) = rest.split_once(':')?;
    if id.is_empty() {
        return None;
    }
    Some((coll, id))
}

fn user_subtitle(
    permalink: Option<&str>,
    city: Option<&str>,
    country: Option<&str>,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(p) = permalink.filter(|p| !p.is_empty()) {
        parts.push(format!("@{p}"));
    }
    let loc: Vec<&str> = [city, country]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    if !loc.is_empty() {
        parts.push(loc.join(", "));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

fn card_from_sc_user(id: &str, v: &Value) -> Option<EntityCard> {
    let username = str_field(v, "username").or_else(|| str_field(v, "full_name"))?;
    let permalink = str_field(v, "permalink");
    let city = str_field(v, "city");
    let country = str_field(v, "country_code").or_else(|| str_field(v, "country"));
    Some(EntityCard {
        kind: "user",
        id: id.to_string(),
        urn: format!("soundcloud:users:{id}"),
        title: username,
        subtitle: user_subtitle(permalink.as_deref(), city.as_deref(), country.as_deref()),
        image: str_field(v, "avatar_url"),
        permalink_url: str_field(v, "permalink_url"),
        verified: v.get("verified").and_then(Value::as_bool).unwrap_or(false),
    })
}

#[tracing::instrument(skip_all)]
pub async fn hydrate(
    _: AdminAuth,
    State(st): State<AppState>,
    Json(body): Json<HydrateReq>,
) -> AppResult<Json<HashMap<String, EntityCard>>> {
    let mut user_ids: HashSet<String> = HashSet::new();
    let mut track_ids: HashSet<String> = HashSet::new();
    let mut playlist_ids: HashSet<String> = HashSet::new();
    let mut ref_targets: Vec<(String, &'static str, String)> = Vec::new();

    for raw in body.refs.into_iter().take(MAX_REFS) {
        let Some((coll, id)) = parse_urn(&raw) else {
            continue;
        };
        let id = id.to_string();
        match coll {
            "users" => {
                user_ids.insert(id.clone());
                ref_targets.push((raw, "user", id));
            }
            "tracks" => {
                track_ids.insert(id.clone());
                ref_targets.push((raw, "track", id));
            }
            "playlists" | "system-playlists" => {
                playlist_ids.insert(id.clone());
                ref_targets.push((raw, "playlist", id));
            }
            _ => {}
        }
    }

    let mut users: HashMap<String, EntityCard> = HashMap::new();
    let mut tracks: HashMap<String, EntityCard> = HashMap::new();
    let mut playlists: HashMap<String, EntityCard> = HashMap::new();

    if !user_ids.is_empty() {
        let ids: Vec<String> = user_ids.iter().cloned().collect();
        let rows =
            sqlx::query_file_as!(UserCardRow, "queries/admin/hydrate/users_by_ids.sql", &ids)
                .fetch_all(&st.pg)
                .await?;
        for r in rows {
            users.insert(
                r.sc_user_id.clone(),
                EntityCard {
                    kind: "user",
                    id: r.sc_user_id,
                    urn: r.urn,
                    title: r.username,
                    subtitle: user_subtitle(
                        r.permalink.as_deref(),
                        r.city.as_deref(),
                        r.country.as_deref(),
                    )
                    .or_else(|| r.followers_count.map(|c| format!("{c} подписчиков"))),
                    image: r.avatar_url,
                    permalink_url: None,
                    verified: r.verified,
                },
            );
        }
    }

    if !track_ids.is_empty() {
        let ids: Vec<String> = track_ids.into_iter().collect();
        let rows = sqlx::query_file_as!(
            TrackCardRow,
            "queries/admin/hydrate/tracks_by_ids.sql",
            &ids
        )
        .fetch_all(&st.pg)
        .await?;
        for r in rows {
            tracks.insert(
                r.sc_track_id.clone(),
                EntityCard {
                    kind: "track",
                    id: r.sc_track_id,
                    urn: r.urn,
                    title: r.title,
                    subtitle: r.uploader_username,
                    image: r.artwork_url.or(r.uploader_avatar_url),
                    permalink_url: r.permalink_url,
                    verified: false,
                },
            );
        }
    }

    if !playlist_ids.is_empty() {
        let ids: Vec<String> = playlist_ids.into_iter().collect();
        let rows = sqlx::query_file_as!(
            PlaylistCardRow,
            "queries/admin/hydrate/playlists_by_ids.sql",
            &ids
        )
        .fetch_all(&st.pg)
        .await?;
        for r in rows {
            playlists.insert(
                r.sc_playlist_id.clone(),
                EntityCard {
                    kind: "playlist",
                    id: r.sc_playlist_id.clone(),
                    urn: format!("soundcloud:playlists:{}", r.sc_playlist_id),
                    title: r.title,
                    subtitle: None,
                    image: r.artwork_url,
                    permalink_url: r.permalink_url,
                    verified: false,
                },
            );
        }
    }

    let missing: Vec<String> = user_ids
        .into_iter()
        .filter(|id| !users.contains_key(id))
        .take(MAX_LIVE_USER_LOOKUPS)
        .collect();
    if !missing.is_empty() {
        let sem = Arc::new(Semaphore::new(LIVE_LOOKUP_CONCURRENCY));
        let fetched = join_all(missing.into_iter().map(|id| {
            let st = st.clone();
            let sem = sem.clone();
            async move {
                let _permit = sem.acquire().await.ok()?;
                let observation = catalog_ingest::Observation::begin(&st.pg).await.ok()?;
                let v = st
                    .resolve
                    .user_by_id(TokenKind::PublicPool, &id)
                    .await
                    .ok()?;
                if let Err(error) = crate::modules::users::UserRepository::new(st.pg.clone())
                    .upsert_from_sc(&v, observation)
                    .await
                {
                    tracing::warn!(user = %id, %error, "hydrated user could not be persisted");
                }
                card_from_sc_user(&id, &v)
            }
        }))
        .await;
        for card in fetched.into_iter().flatten() {
            users.insert(card.id.clone(), card);
        }
    }

    let mut out: HashMap<String, EntityCard> = HashMap::new();
    for (raw, kind, id) in ref_targets {
        let card = match kind {
            "user" => users.get(&id),
            "track" => tracks.get(&id),
            "playlist" => playlists.get(&id),
            _ => None,
        };
        if let Some(c) = card {
            out.insert(raw, c.clone());
        }
    }
    Ok(Json(out))
}

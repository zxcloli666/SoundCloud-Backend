use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::error::EnrichResult;
use super::resolver::{AlbumCandidate, ArtistCandidate, Credit, ResolveResult, ResolveSource};
use catalog_normalize::{clean_artist_name, normalize_name, normalize_title};

pub struct PersistOutcome {
    pub primary_artist_id: Option<Uuid>,
    pub album_id: Option<Uuid>,
    pub coplay_dirty: bool,
}

pub async fn apply(
    pg: &PgPool,
    track_id: Uuid,
    res: &ResolveResult,
    uploader_sc_user_id: Option<&str>,
    uploader_username: Option<&str>,
) -> EnrichResult<PersistOutcome> {
    let mut tx = pg.begin().await?;

    let primary_ids = upsert_artists(&mut tx, &res.primary).await?;
    let featured_ids = upsert_artists(&mut tx, &res.featured).await?;
    let producer_ids = upsert_artists(&mut tx, &res.producers).await?;
    let remixer_ids = upsert_artists(&mut tx, &res.remixers).await?;

    let prior_count: i64 =
        sqlx::query_file_scalar!("queries/enrich/persist/count_track_artists.sql", track_id)
            .fetch_one(&mut *tx)
            .await?;

    sqlx::query_file!("queries/enrich/persist/delete_track_artists.sql", track_id)
        .execute(&mut *tx)
        .await?;

    let mut all_artist_ids: Vec<Uuid> = Vec::new();
    if !res.is_cover {
        insert_track_artists(
            &mut tx,
            track_id,
            &primary_ids,
            "primary",
            &mut all_artist_ids,
        )
        .await?;
        insert_track_artists(
            &mut tx,
            track_id,
            &featured_ids,
            "featured",
            &mut all_artist_ids,
        )
        .await?;
        insert_track_artists(
            &mut tx,
            track_id,
            &producer_ids,
            "producer",
            &mut all_artist_ids,
        )
        .await?;
        insert_track_artists(
            &mut tx,
            track_id,
            &remixer_ids,
            "remixer",
            &mut all_artist_ids,
        )
        .await?;
    } else {
        let _ = (&featured_ids, &producer_ids, &remixer_ids);
    }

    let leading_primary = primary_ids.first().map(|(id, _)| *id);
    let (primary_artist_id, cover_of_artist_id) = if res.is_cover {
        (None, leading_primary)
    } else {
        (leading_primary, None)
    };

    let album_id = if let Some(album) = res.album.as_ref() {
        Some(upsert_album(&mut tx, album, res.source, res.confidence).await?)
    } else {
        None
    };

    if let (Some(album_id), Some(_)) = (album_id, primary_artist_id) {
        link_album_track(&mut tx, album_id, track_id).await?;
    }

    let canonical_id = match res.isrc.as_deref() {
        Some(isrc) if !isrc.is_empty() => {
            Some(resolve_canonical_for_isrc(&mut tx, track_id, isrc).await?)
        }
        _ => None,
    };

    if let (Some(artist_id), Some(sc_id)) = (primary_artist_id, uploader_sc_user_id) {
        sqlx::query_file!(
            "queries/enrich/persist/set_uploader_sc_user_id.sql",
            track_id,
            sc_id
        )
        .execute(&mut *tx)
        .await?;

        let primary_name = res.primary.first().map(|c| c.name.as_str()).unwrap_or("");
        maybe_auto_attach_sc_account(
            &mut tx,
            artist_id,
            sc_id,
            uploader_username.unwrap_or(""),
            primary_name,
        )
        .await?;

        maybe_attach_reupload_account(&mut tx, artist_id, sc_id).await?;
    }

    let upload_kind = if res.is_cover {
        "cover"
    } else {
        compute_upload_kind(&mut tx, primary_artist_id, uploader_sc_user_id, res.source).await?
    };
    let is_cover = matches!(upload_kind, "cover" | "reupload") || cover_of_artist_id.is_some();

    let source = res.source.as_str();
    let confidence = calibrate_confidence(&mut tx, source, res.confidence).await?;
    sqlx::query_file!(
        "queries/enrich/persist/finalize_track.sql",
        track_id,
        primary_artist_id,
        album_id,
        res.isrc.as_deref(),
        canonical_id,
        cover_of_artist_id,
        source,
        confidence,
        upload_kind,
        res.release_date,
        res.release_year,
        is_cover,
        res.genius_song_id,
        res.genius_url.as_deref()
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(PersistOutcome {
        primary_artist_id,
        album_id,
        coplay_dirty: prior_count == 0 && all_artist_ids.len() >= 2,
    })
}

async fn calibrate_confidence(
    tx: &mut Transaction<'_, Postgres>,
    source: &str,
    raw: f32,
) -> EnrichResult<f32> {
    let row: Option<f32> = sqlx::query_file_scalar!(
        "queries/enrich/persist/calibrated_confidence.sql",
        source,
        raw
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|v| v.clamp(0.0, 1.0)).unwrap_or(raw))
}

async fn maybe_auto_attach_sc_account(
    tx: &mut Transaction<'_, Postgres>,
    artist_id: Uuid,
    sc_user_id: &str,
    uploader_username: &str,
    artist_name: &str,
) -> EnrichResult<()> {
    if sc_user_id.is_empty() || uploader_username.is_empty() || artist_name.is_empty() {
        return Ok(());
    }
    let exists: Option<String> = sqlx::query_file_scalar!(
        "queries/enrich/persist/sc_account_role.sql",
        artist_id,
        sc_user_id
    )
    .fetch_optional(&mut **tx)
    .await?;
    if exists.is_some() {
        return Ok(());
    }
    let un = normalize_name(uploader_username);
    let an = normalize_name(artist_name);
    if un.is_empty() || an.is_empty() {
        return Ok(());
    }
    let exact = un == an;
    let strong_substring = un.len() >= 4 && an.len() >= 4 && (un.contains(&an) || an.contains(&un));
    if !exact && !strong_substring {
        return Ok(());
    }
    let has_main: Option<i64> = sqlx::query_file_scalar!(
        "queries/enrich/persist/count_main_sc_accounts.sql",
        artist_id
    )
    .fetch_optional(&mut **tx)
    .await?;
    let role = match has_main {
        Some(n) if n == 0 && exact => "main",
        _ => "alt",
    };
    sqlx::query_file!(
        "queries/enrich/persist/insert_auto_match_account.sql",
        artist_id,
        sc_user_id,
        role
    )
    .execute(&mut **tx)
    .await?;
    sqlx::query_file!(
        "queries/enrich/persist/set_artist_sc_user_id.sql",
        artist_id,
        sc_user_id
    )
    .execute(&mut **tx)
    .await?;
    sqlx::query_file!(
        "queries/enrich/persist/repoint_uploader_tracks.sql",
        sc_user_id,
        artist_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

const REUPLOAD_THRESHOLD: i64 = 3;

async fn maybe_attach_reupload_account(
    tx: &mut Transaction<'_, Postgres>,
    artist_id: Uuid,
    sc_user_id: &str,
) -> EnrichResult<()> {
    if sc_user_id.is_empty() {
        return Ok(());
    }
    let exists: Option<i32> = sqlx::query_file_scalar!(
        "queries/enrich/persist/sc_account_exists.sql",
        artist_id,
        sc_user_id
    )
    .fetch_optional(&mut **tx)
    .await?;
    if exists.is_some() {
        return Ok(());
    }
    let count: i64 = sqlx::query_file_scalar!(
        "queries/enrich/persist/count_reupload_tracks.sql",
        sc_user_id,
        artist_id
    )
    .fetch_one(&mut **tx)
    .await?;
    if count < REUPLOAD_THRESHOLD {
        return Ok(());
    }
    sqlx::query_file!(
        "queries/enrich/persist/insert_reupload_account.sql",
        artist_id,
        sc_user_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn compute_upload_kind(
    tx: &mut Transaction<'_, Postgres>,
    primary_artist_id: Option<Uuid>,
    uploader_sc_user_id: Option<&str>,
    source: ResolveSource,
) -> EnrichResult<&'static str> {
    let Some(artist_id) = primary_artist_id else {
        return Ok("unknown");
    };
    if let Some(sc_id) = uploader_sc_user_id
        && !sc_id.is_empty()
    {
        let account = sqlx::query_file!(
            "queries/enrich/persist/sc_account_role_verified.sql",
            artist_id,
            sc_id
        )
        .fetch_optional(&mut **tx)
        .await?
        .filter(|account| account.verified || account.source == "mb_resolve");
        if let Some(account) = account {
            return Ok(match account.role.as_str() {
                "main" => "original",
                "demo" => "demo",
                _ => "alt",
            });
        }
    }
    let external_credit = matches!(
        source,
        ResolveSource::Isrc | ResolveSource::Mb | ResolveSource::Genius | ResolveSource::ScVerified
    );
    Ok(if external_credit {
        "reupload"
    } else {
        "unknown"
    })
}

async fn upsert_artists(
    tx: &mut Transaction<'_, Postgres>,
    candidates: &[ArtistCandidate],
) -> EnrichResult<Vec<(Uuid, Credit)>> {
    let mut credited: Vec<(Uuid, Credit)> = Vec::with_capacity(candidates.len());
    for c in candidates {
        let cleaned = clean_artist_name(&c.name);
        if cleaned.is_empty() {
            continue;
        }
        let normalized = normalize_name(&cleaned);
        if normalized.is_empty() {
            continue;
        }
        let credit = c.credit();
        let id = upsert_one_artist(
            tx,
            &cleaned,
            &normalized,
            c,
            credit.source,
            credit.confidence,
        )
        .await?;
        match credited.iter_mut().find(|(known, _)| *known == id) {
            Some((_, held)) => {
                if stronger(&credit, held) {
                    *held = credit;
                }
            }
            None => credited.push((id, credit)),
        }
    }
    Ok(credited)
}

fn stronger(candidate: &Credit, held: &Credit) -> bool {
    let candidate_rank = candidate.source.priority();
    let held_rank = held.source.priority();
    candidate_rank > held_rank
        || (candidate_rank == held_rank && candidate.confidence > held.confidence)
}

async fn upsert_one_artist(
    tx: &mut Transaction<'_, Postgres>,
    name: &str,
    normalized: &str,
    cand: &ArtistCandidate,
    source: ResolveSource,
    confidence: f32,
) -> EnrichResult<Uuid> {
    if let Some(mb_id) = cand.mb_id.as_deref() {
        let existing: Option<Uuid> =
            sqlx::query_file_scalar!("queries/enrich/persist/artist_by_mb_id.sql", mb_id)
                .fetch_optional(&mut **tx)
                .await?;
        if let Some(id) = existing {
            maybe_promote(tx, id, cand, source, confidence).await?;
            return resolve_merged(tx, id).await;
        }
    }

    if let Some(genius_id) = cand.genius_id.as_deref() {
        let existing: Option<Uuid> =
            sqlx::query_file_scalar!("queries/enrich/persist/artist_by_genius_id.sql", genius_id)
                .fetch_optional(&mut **tx)
                .await?;
        if let Some(id) = existing {
            maybe_promote(tx, id, cand, source, confidence).await?;
            return resolve_merged(tx, id).await;
        }
    }

    let existing: Option<Uuid> = sqlx::query_file_scalar!(
        "queries/enrich/persist/artist_by_normalized_name.sql",
        normalized
    )
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(id) = existing {
        maybe_promote(tx, id, cand, source, confidence).await?;
        return resolve_merged(tx, id).await;
    }

    let inserted: (Uuid,) = sqlx::query_as(
        "INSERT INTO artists (name, normalized_name, mb_artist_id, genius_artist_id, sc_user_id, source, confidence)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id",
    )
    .bind(name)
    .bind(normalized)
    .bind(cand.mb_id.as_deref())
    .bind(cand.genius_id.as_deref())
    .bind(cand.sc_user_id.as_deref())
    .bind(source.as_str())
    .bind(confidence)
    .fetch_one(&mut **tx)
    .await?;
    Ok(inserted.0)
}

async fn maybe_promote(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    cand: &ArtistCandidate,
    source: ResolveSource,
    confidence: f32,
) -> EnrichResult<()> {
    let row = sqlx::query_file!("queries/enrich/persist/artist_promote_row.sql", id)
        .fetch_optional(&mut **tx)
        .await?;
    let Some(r) = row else {
        return Ok(());
    };
    let (cur_source, cur_conf, cur_mb, cur_genius, cur_sc) = (
        r.source,
        r.confidence,
        r.mb_artist_id,
        r.genius_artist_id,
        r.sc_user_id,
    );
    let new_priority = source.priority();
    let cur_priority = ResolveSource::priority_of(&cur_source);
    let stronger = new_priority > cur_priority
        || (new_priority == cur_priority && confidence > cur_conf + 0.05);
    let mb_to_set = cand.mb_id.clone().or(cur_mb);
    let genius_to_set = cand.genius_id.clone().or(cur_genius);
    let sc_to_set = cand.sc_user_id.clone().or(cur_sc);
    if !stronger && mb_to_set.is_none() && genius_to_set.is_none() && sc_to_set.is_none() {
        return Ok(());
    }
    sqlx::query_file!(
        "queries/enrich/persist/promote_artist.sql",
        id,
        mb_to_set.as_deref(),
        genius_to_set.as_deref(),
        sc_to_set.as_deref(),
        stronger,
        source.as_str(),
        confidence
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn resolve_merged(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> EnrichResult<Uuid> {
    let mut current = id;
    for _ in 0..4 {
        let next: Option<Option<Uuid>> =
            sqlx::query_file_scalar!("queries/enrich/persist/artist_merged_into.sql", current)
                .fetch_optional(&mut **tx)
                .await?;
        match next {
            Some(Some(parent)) => current = parent,
            _ => break,
        }
    }
    Ok(current)
}

async fn insert_track_artists(
    tx: &mut Transaction<'_, Postgres>,
    track_id: Uuid,
    credited: &[(Uuid, Credit)],
    role: &str,
    accum: &mut Vec<Uuid>,
) -> EnrichResult<()> {
    for (pos, (id, credit)) in credited.iter().enumerate() {
        sqlx::query_file!(
            "queries/enrich/persist/insert_track_artist.sql",
            track_id,
            id,
            role,
            pos as i16,
            credit.source.as_str(),
            credit.confidence,
            credit.evidence.as_str()
        )
        .execute(&mut **tx)
        .await?;
        if !accum.contains(id) {
            accum.push(*id);
        }
    }
    Ok(())
}

async fn upsert_album(
    tx: &mut Transaction<'_, Postgres>,
    album: &AlbumCandidate,
    source: ResolveSource,
    confidence: f32,
) -> EnrichResult<Uuid> {
    if let Some(mb_id) = album.mb_id.as_deref() {
        let existing: Option<Uuid> =
            sqlx::query_file_scalar!("queries/enrich/persist/album_by_mb_id.sql", mb_id)
                .fetch_optional(&mut **tx)
                .await?;
        if let Some(id) = existing {
            if let Some(cover) = album.cover_url.as_deref() {
                sqlx::query_file!(
                    "queries/enrich/persist/album_fill_cover_mb.sql",
                    id,
                    cover,
                    album.year
                )
                .execute(&mut **tx)
                .await?;
            }
            return Ok(id);
        }
    }
    if let Some(g_id) = album.genius_id.as_deref() {
        let existing: Option<Uuid> =
            sqlx::query_file_scalar!("queries/enrich/persist/album_by_genius_id.sql", g_id)
                .fetch_optional(&mut **tx)
                .await?;
        if let Some(id) = existing {
            sqlx::query_file!(
                "queries/enrich/persist/album_fill_cover_genius.sql",
                id,
                album.cover_url.as_deref(),
                album.year
            )
            .execute(&mut **tx)
            .await?;
            return Ok(id);
        }
    }

    let primary_artist_id = if let Some(pa) = album.primary_artist.as_ref() {
        let n = normalize_name(&pa.name);
        if n.is_empty() {
            None
        } else {
            let id = upsert_one_artist(tx, pa.name.trim(), &n, pa, source, confidence).await?;
            Some(id)
        }
    } else {
        None
    };

    let normalized_title = normalize_title(&album.title);
    let kind = match album.release_type.as_deref() {
        Some("EP") => "ep",
        Some("Single") => "single",
        Some("Compilation") => "compilation",
        _ => "album",
    };

    let inserted: (Uuid,) = sqlx::query_as(
        "INSERT INTO albums (title, normalized_title, primary_artist_id, type, release_year, mb_release_id, genius_album_id, cover_url, source, confidence)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING id",
    )
    .bind(album.title.trim())
    .bind(&normalized_title)
    .bind(primary_artist_id)
    .bind(kind)
    .bind(album.year)
    .bind(album.mb_id.as_deref())
    .bind(album.genius_id.as_deref())
    .bind(album.cover_url.as_deref())
    .bind(source.as_str())
    .bind(confidence)
    .fetch_one(&mut **tx)
    .await?;

    if let Some(pa_id) = primary_artist_id {
        sqlx::query_file!(
            "queries/enrich/persist/insert_album_artist.sql",
            inserted.0,
            pa_id
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(inserted.0)
}

async fn link_album_track(
    tx: &mut Transaction<'_, Postgres>,
    album_id: Uuid,
    track_id: Uuid,
) -> EnrichResult<()> {
    sqlx::query_file!(
        "queries/enrich/persist/insert_album_track.sql",
        album_id,
        track_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn resolve_canonical_for_isrc(
    tx: &mut Transaction<'_, Postgres>,
    track_id: Uuid,
    isrc: &str,
) -> EnrichResult<Uuid> {
    sqlx::query_file!("queries/enrich/persist/isrc_advisory_lock.sql", isrc)
        .execute(&mut **tx)
        .await?;
    let existing: Option<Uuid> = sqlx::query_file_scalar!(
        "queries/enrich/persist/existing_canonical_for_isrc.sql",
        isrc,
        track_id
    )
    .fetch_optional(&mut **tx)
    .await?;
    match existing {
        Some(cid) => Ok(cid),
        None => {
            let new_id = Uuid::new_v4();
            sqlx::query_file!(
                "queries/enrich/persist/assign_canonical_for_isrc.sql",
                new_id,
                isrc
            )
            .execute(&mut **tx)
            .await?;
            Ok(new_id)
        }
    }
}

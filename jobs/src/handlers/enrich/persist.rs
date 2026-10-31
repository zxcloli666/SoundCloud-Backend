mod accounts;

#[cfg(test)]
#[path = "persist_tests.rs"]
mod tests;

use sqlx::{Acquire, PgConnection, PgPool, Postgres, Transaction};
use tracing::debug;
use uuid::Uuid;

use self::accounts::{
    compute_upload_kind, maybe_attach_reupload_account, maybe_auto_attach_sc_account,
    without_foreign_namesakes,
};
use super::error::{EnrichError, EnrichResult};
use super::resolver::{AlbumCandidate, ArtistCandidate, Credit, ResolveResult, ResolveSource};
use crate::handlers::artist_names::{MIN_NAME_LENGTH, name_length};
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

    let primaries = if res.is_cover {
        res.primary.clone()
    } else {
        without_foreign_namesakes(
            &mut tx,
            &res.primary,
            uploader_sc_user_id,
            uploader_username,
        )
        .await?
    };
    let [primary_ids, featured_ids, producer_ids, remixer_ids] = upsert_artists(
        &mut tx,
        [&primaries, &res.featured, &res.producers, &res.remixers],
        uploader_sc_user_id,
    )
    .await?;

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
        Some(
            upsert_album(
                &mut tx,
                album,
                res.source,
                res.confidence,
                uploader_sc_user_id,
            )
            .await?,
        )
    } else {
        None
    };

    if let (Some(album_id), Some(_)) = (album_id, primary_artist_id) {
        link_album_track(&mut tx, album_id, track_id).await?;
    }

    if let (Some(artist_id), Some(sc_id)) = (primary_artist_id, uploader_sc_user_id) {
        let primary_name = primaries.first().map(|c| c.name.as_str()).unwrap_or("");
        maybe_auto_attach_sc_account(
            &mut tx,
            artist_id,
            sc_id,
            uploader_username.unwrap_or(""),
            primary_name,
        )
        .await?;

        maybe_attach_reupload_account(&mut tx, track_id, artist_id, sc_id).await?;
    }

    let upload_kind = if res.is_cover {
        "cover"
    } else {
        compute_upload_kind(&mut tx, primary_artist_id, uploader_sc_user_id, res.source).await?
    };
    let is_cover = matches!(upload_kind, "cover" | "reupload") || cover_of_artist_id.is_some();

    let source = res.source.as_str();
    let confidence = calibrate_confidence(&mut tx, source, res.confidence).await?;

    let canonical_id = match res.isrc.as_deref() {
        Some(isrc) if !isrc.is_empty() => {
            Some(resolve_canonical_for_isrc(&mut tx, track_id, isrc).await?)
        }
        _ => None,
    };

    if let (Some(_), Some(sc_id)) = (primary_artist_id, uploader_sc_user_id) {
        sqlx::query_file!(
            "queries/enrich/persist/set_uploader_sc_user_id.sql",
            track_id,
            sc_id
        )
        .execute(&mut *tx)
        .await?;
    }

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

async fn upsert_artists(
    tx: &mut Transaction<'_, Postgres>,
    groups: [&[ArtistCandidate]; 4],
    uploader_sc_user_id: Option<&str>,
) -> EnrichResult<[Vec<(Uuid, Credit)>; 4]> {
    let mut ordered: Vec<(String, String, usize, usize)> = Vec::new();
    for (group, candidates) in groups.iter().enumerate() {
        for (position, c) in candidates.iter().enumerate() {
            let cleaned = clean_artist_name(&c.name);
            if cleaned.is_empty() {
                continue;
            }
            let normalized = normalize_name(&cleaned);
            if normalized.is_empty() {
                continue;
            }
            ordered.push((normalized, cleaned, group, position));
        }
    }
    ordered.sort_by(|a, b| a.0.cmp(&b.0));

    let mut resolved: Vec<Vec<Option<Uuid>>> = groups.iter().map(|g| vec![None; g.len()]).collect();
    for (normalized, cleaned, group, position) in ordered {
        let c = &groups[group][position];
        let credit = c.credit();
        resolved[group][position] = upsert_one_artist(
            tx,
            &cleaned,
            &normalized,
            c,
            credit.source,
            credit.confidence,
            uploader_sc_user_id,
        )
        .await?;
    }

    Ok(std::array::from_fn(|group| {
        let mut credited: Vec<(Uuid, Credit)> = Vec::with_capacity(groups[group].len());
        for (c, id) in groups[group].iter().zip(&resolved[group]) {
            let Some(id) = *id else {
                continue;
            };
            let credit = c.credit();
            match credited.iter_mut().find(|(known, _)| *known == id) {
                Some((_, held)) => {
                    if stronger(&credit, held) {
                        *held = credit;
                    }
                }
                None => credited.push((id, credit)),
            }
        }
        credited
    }))
}

fn stronger(candidate: &Credit, held: &Credit) -> bool {
    let candidate_rank = candidate.source.priority();
    let held_rank = held.source.priority();
    candidate_rank > held_rank
        || (candidate_rank == held_rank && candidate.confidence > held.confidence)
}

enum Attempt {
    Settled(Option<Uuid>),
    Conflict,
}

async fn upsert_one_artist(
    tx: &mut Transaction<'_, Postgres>,
    name: &str,
    normalized: &str,
    cand: &ArtistCandidate,
    source: ResolveSource,
    confidence: f32,
    uploader_sc_user_id: Option<&str>,
) -> EnrichResult<Option<Uuid>> {
    for _ in 0..3 {
        let attempt = try_upsert_one_artist(
            tx,
            name,
            normalized,
            cand,
            source,
            confidence,
            uploader_sc_user_id,
        )
        .await?;
        if let Attempt::Settled(id) = attempt {
            return Ok(id);
        }
    }
    Err(EnrichError::rejected(format!(
        "artist '{normalized}' kept conflicting on insert"
    )))
}

async fn try_upsert_one_artist(
    tx: &mut Transaction<'_, Postgres>,
    name: &str,
    normalized: &str,
    cand: &ArtistCandidate,
    source: ResolveSource,
    confidence: f32,
    uploader_sc_user_id: Option<&str>,
) -> EnrichResult<Attempt> {
    if let Some(mb_id) = cand.mb_id.as_deref() {
        let existing: Option<Uuid> =
            sqlx::query_file_scalar!("queries/enrich/persist/artist_by_mb_id.sql", mb_id)
                .fetch_optional(&mut **tx)
                .await?;
        if let Some(id) = existing {
            maybe_promote(tx, id, cand, source, confidence).await?;
            return resolve_merged(tx, id)
                .await
                .map(|id| Attempt::Settled(Some(id)));
        }
    }

    if let Some(genius_id) = cand.genius_id.as_deref() {
        let existing: Option<Uuid> =
            sqlx::query_file_scalar!("queries/enrich/persist/artist_by_genius_id.sql", genius_id)
                .fetch_optional(&mut **tx)
                .await?;
        if let Some(id) = existing {
            maybe_promote(tx, id, cand, source, confidence).await?;
            return resolve_merged(tx, id)
                .await
                .map(|id| Attempt::Settled(Some(id)));
        }
    }

    let existing: Option<Uuid> = sqlx::query_file_scalar!(
        "queries/enrich/persist/artist_by_normalized_name.sql",
        normalized
    )
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(id) = existing {
        let external_id_unmatched = cand.mb_id.is_some() || cand.genius_id.is_some();
        if external_id_unmatched
            && name_length(normalized) < MIN_NAME_LENGTH
            && sqlx::query_file_scalar!(
                "queries/enrich/persist/name_match_is_another_artist.sql",
                id,
                cand.mb_id.as_deref(),
                cand.genius_id.as_deref(),
                uploader_sc_user_id
            )
            .fetch_one(&mut **tx)
            .await?
        {
            debug!(
                artist = %name,
                uploader = ?uploader_sc_user_id,
                "external artist kept off a same-named artist with its own identity"
            );
            return Ok(Attempt::Settled(None));
        }
        maybe_promote(tx, id, cand, source, confidence).await?;
        return resolve_merged(tx, id)
            .await
            .map(|id| Attempt::Settled(Some(id)));
    }

    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO artists (name, normalized_name, mb_artist_id, genius_artist_id, source, confidence)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT DO NOTHING
         RETURNING id",
    )
    .bind(name)
    .bind(normalized)
    .bind(cand.mb_id.as_deref())
    .bind(cand.genius_id.as_deref())
    .bind(source.as_str())
    .bind(confidence)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(match inserted {
        Some(id) => Attempt::Settled(Some(id)),
        None => Attempt::Conflict,
    })
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
    let (cur_source, cur_conf, cur_mb, cur_genius) =
        (r.source, r.confidence, r.mb_artist_id, r.genius_artist_id);
    let new_priority = source.priority();
    let cur_priority = ResolveSource::priority_of(&cur_source);
    let stronger = new_priority > cur_priority
        || (new_priority == cur_priority && confidence > cur_conf + 0.05);
    let mb_to_set = cand.mb_id.clone().or(cur_mb.clone());
    let genius_to_set = cand.genius_id.clone().or(cur_genius.clone());
    if !stronger && mb_to_set.is_none() && genius_to_set.is_none() {
        return Ok(());
    }
    let takes_new_id = (cand.mb_id.is_some() && cur_mb.is_none())
        || (cand.genius_id.is_some() && cur_genius.is_none());
    if takes_new_id {
        let mut savepoint = tx.begin().await?;
        let written = write_promotion(
            &mut savepoint,
            id,
            mb_to_set.as_deref(),
            genius_to_set.as_deref(),
            stronger,
            source,
            confidence,
        )
        .await;
        match written {
            Ok(()) => return Ok(savepoint.commit().await?),
            Err(error) if is_unique_violation(&error) => {
                savepoint.rollback().await?;
                debug!(artist = %id, "external id was taken by another artist, link kept without it");
            }
            Err(error) => return Err(error.into()),
        }
    }
    write_promotion(
        tx,
        id,
        cur_mb.as_deref(),
        cur_genius.as_deref(),
        stronger,
        source,
        confidence,
    )
    .await?;
    Ok(())
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|error| error.is_unique_violation())
}

async fn write_promotion(
    conn: &mut PgConnection,
    id: Uuid,
    mb_artist_id: Option<&str>,
    genius_artist_id: Option<&str>,
    stronger: bool,
    source: ResolveSource,
    confidence: f32,
) -> Result<(), sqlx::Error> {
    sqlx::query_file!(
        "queries/enrich/persist/promote_artist.sql",
        id,
        mb_artist_id,
        genius_artist_id,
        stronger,
        source.as_str(),
        confidence
    )
    .execute(conn)
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
    uploader_sc_user_id: Option<&str>,
) -> EnrichResult<Uuid> {
    if let Some(id) = existing_album_by_mb_id(tx, album).await? {
        return Ok(id);
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
            upsert_one_artist(
                tx,
                pa.name.trim(),
                &n,
                pa,
                source,
                confidence,
                uploader_sc_user_id,
            )
            .await?
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

    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO albums (title, normalized_title, primary_artist_id, type, release_year, mb_release_id, genius_album_id, cover_url, source, confidence)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT DO NOTHING
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
    .fetch_optional(&mut **tx)
    .await?;
    let Some(album_id) = inserted else {
        return match existing_album_by_mb_id(tx, album).await? {
            Some(id) => Ok(id),
            None => Err(EnrichError::rejected(format!(
                "album '{}' kept conflicting on insert",
                album.title
            ))),
        };
    };

    if let Some(pa_id) = primary_artist_id {
        sqlx::query_file!(
            "queries/enrich/persist/insert_album_artist.sql",
            album_id,
            pa_id
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(album_id)
}

async fn existing_album_by_mb_id(
    tx: &mut Transaction<'_, Postgres>,
    album: &AlbumCandidate,
) -> EnrichResult<Option<Uuid>> {
    let Some(mb_id) = album.mb_id.as_deref() else {
        return Ok(None);
    };
    let existing: Option<Uuid> =
        sqlx::query_file_scalar!("queries/enrich/persist/album_by_mb_id.sql", mb_id)
            .fetch_optional(&mut **tx)
            .await?;
    let Some(id) = existing else {
        return Ok(None);
    };
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
    Ok(Some(id))
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

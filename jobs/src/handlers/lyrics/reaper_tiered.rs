use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};

use super::TranscriptionCandidate;
use crate::handlers::star_priority::{
    LIKED_PRIORITY, Pass, Round, STAR_SCAN_WINDOW, Tier, Turn, of_tracks, split_next, with_next,
};

const LIKED_SCAN_WINDOW: i64 = 10_000;

struct EmbeddingCandidate {
    sc_track_id: String,
    waiting_since: DateTime<Utc>,
}

struct AlignCandidate {
    sc_track_id: String,
    uploaded_generation: i64,
    waiting_since: DateTime<Utc>,
}

pub(super) async fn embeddings(
    transaction: &mut Transaction<'_, Postgres>,
    room: i64,
    turn: Turn,
) -> Result<Vec<(String, Tier)>, sqlx::Error> {
    if room <= 0 {
        return Ok(Vec::new());
    }
    let room = usize::try_from(room).unwrap_or(0);
    let mut round = Round::new("lyrics_embed");
    let mut picked: Vec<String> = Vec::new();
    for tier in turn.tiers() {
        let left = room.saturating_sub(picked.len());
        let found = embedding_pass(transaction, tier.pass(), left, &picked).await?;
        let (found, next) = split_next(found, left);
        round.record(
            tier.label(),
            found.len(),
            next.map(|candidate| candidate.waiting_since),
        );
        picked.extend(found.into_iter().map(|candidate| candidate.sc_track_id));
    }
    round.report();
    let tiers = of_tracks(&mut **transaction, &picked).await?;
    Ok(picked
        .into_iter()
        .map(|sc_track_id| {
            let tier = tiers.get(&sc_track_id).copied().unwrap_or(Tier::Rest);
            (sc_track_id, tier)
        })
        .collect())
}

pub(super) async fn alignments(
    transaction: &mut Transaction<'_, Postgres>,
    room: i64,
    turn: Turn,
) -> Result<Vec<(TranscriptionCandidate, Tier)>, sqlx::Error> {
    if room <= 0 {
        return Ok(Vec::new());
    }
    let room = usize::try_from(room).unwrap_or(0);
    let mut round = Round::new("lyrics_align");
    let mut picked: Vec<TranscriptionCandidate> = Vec::new();
    for tier in turn.tiers() {
        let left = room.saturating_sub(picked.len());
        let taken: Vec<String> = picked
            .iter()
            .map(|candidate| candidate.sc_track_id.clone())
            .collect();
        let found = align_pass(transaction, tier.pass(), left, &taken).await?;
        let (found, next) = split_next(found, left);
        round.record(
            tier.label(),
            found.len(),
            next.map(|candidate| candidate.waiting_since),
        );
        picked.extend(found.into_iter().map(|found| TranscriptionCandidate {
            sc_track_id: found.sc_track_id,
            uploaded_generation: found.uploaded_generation,
        }));
    }
    round.report();
    let ids: Vec<String> = picked
        .iter()
        .map(|candidate| candidate.sc_track_id.clone())
        .collect();
    let tiers = of_tracks(&mut **transaction, &ids).await?;
    Ok(picked
        .into_iter()
        .map(|candidate| {
            let tier = tiers
                .get(&candidate.sc_track_id)
                .copied()
                .unwrap_or(Tier::Rest);
            (candidate, tier)
        })
        .collect())
}

async fn embedding_pass(
    transaction: &mut Transaction<'_, Postgres>,
    pass: Pass,
    left: usize,
    taken: &[String],
) -> Result<Vec<EmbeddingCandidate>, sqlx::Error> {
    let limit = with_next(left);
    match pass {
        Pass::Star(stored) => {
            sqlx::query_file_as!(
                EmbeddingCandidate,
                "queries/lyrics/reap_embeddings_star.sql",
                limit,
                stored,
                taken,
                STAR_SCAN_WINDOW
            )
            .fetch_all(&mut **transaction)
            .await
        }
        Pass::Liked => {
            sqlx::query_file_as!(
                EmbeddingCandidate,
                "queries/lyrics/reap_embeddings_liked.sql",
                limit,
                LIKED_PRIORITY,
                taken,
                LIKED_SCAN_WINDOW
            )
            .fetch_all(&mut **transaction)
            .await
        }
        Pass::Rest => {
            sqlx::query_file_as!(
                EmbeddingCandidate,
                "queries/lyrics/reap_embeddings.sql",
                limit,
                taken
            )
            .fetch_all(&mut **transaction)
            .await
        }
    }
}

async fn align_pass(
    transaction: &mut Transaction<'_, Postgres>,
    pass: Pass,
    left: usize,
    taken: &[String],
) -> Result<Vec<AlignCandidate>, sqlx::Error> {
    let limit = with_next(left);
    match pass {
        Pass::Star(stored) => {
            sqlx::query_file_as!(
                AlignCandidate,
                "queries/lyrics/reap_transcriptions_align_star.sql",
                limit,
                stored,
                taken,
                STAR_SCAN_WINDOW
            )
            .fetch_all(&mut **transaction)
            .await
        }
        Pass::Liked => {
            sqlx::query_file_as!(
                AlignCandidate,
                "queries/lyrics/reap_transcriptions_align_liked.sql",
                limit,
                LIKED_PRIORITY,
                taken,
                LIKED_SCAN_WINDOW
            )
            .fetch_all(&mut **transaction)
            .await
        }
        Pass::Rest => {
            sqlx::query_file_as!(
                AlignCandidate,
                "queries/lyrics/reap_transcriptions_align.sql",
                limit,
                taken
            )
            .fetch_all(&mut **transaction)
            .await
        }
    }
}

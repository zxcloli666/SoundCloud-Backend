use backend_contracts::{INDEX_TRACK_PRIORITY, JobKind};
use sqlx::PgPool;

use super::tier::{CORE_FAST_TRACK_PRIORITY, LYRICS_EMBED_PRIORITY, Tier};

const TIERED_KINDS: [(JobKind, i16); 4] = [
    (JobKind::IndexTrack, INDEX_TRACK_PRIORITY),
    (JobKind::LyricsEmbed, LYRICS_EMBED_PRIORITY),
    (JobKind::DispatchAudioIndex, CORE_FAST_TRACK_PRIORITY),
    (JobKind::DispatchTranscription, CORE_FAST_TRACK_PRIORITY),
];

pub(crate) async fn record_queued_waits(pool: &PgPool) -> Result<(), sqlx::Error> {
    let wanted: Vec<(JobKind, Tier, i16)> = TIERED_KINDS
        .iter()
        .flat_map(|&(kind, base)| Tier::ORDER.map(|tier| (kind, tier, tier.job_priority(base))))
        .collect();
    let kinds: Vec<String> = wanted
        .iter()
        .map(|(kind, _, _)| kind.as_str().to_owned())
        .collect();
    let priorities: Vec<i16> = wanted.iter().map(|(_, _, priority)| *priority).collect();
    let rows = sqlx::query_file!(
        "queries/star_priority/queued_waits.sql",
        &kinds,
        &priorities
    )
    .fetch_all(pool)
    .await?;
    for (kind, tier, priority) in wanted {
        let waiting_seconds = rows
            .iter()
            .find(|row| row.kind == kind.as_str() && row.priority == priority)
            .map_or(0.0, |row| row.waiting_seconds);
        crate::metrics::record_star_priority_queued(kind.as_str(), tier.label(), waiting_seconds);
    }
    Ok(())
}

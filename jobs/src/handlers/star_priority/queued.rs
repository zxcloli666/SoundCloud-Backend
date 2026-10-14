use backend_contracts::{
    CATALOG_REFRESH_BACKGROUND_PRIORITY, CATALOG_REFRESH_VIEWER_PRIORITY, INDEX_TRACK_PRIORITY,
    JobKind, STAR_PRIORITY_BONUS,
};
use sqlx::PgPool;

use super::tier::{CORE_FAST_TRACK_PRIORITY, LYRICS_EMBED_PRIORITY, Tier};

const TIERED_KINDS: [(JobKind, i16); 4] = [
    (JobKind::IndexTrack, INDEX_TRACK_PRIORITY),
    (JobKind::LyricsEmbed, LYRICS_EMBED_PRIORITY),
    (JobKind::DispatchAudioIndex, CORE_FAST_TRACK_PRIORITY),
    (JobKind::DispatchTranscription, CORE_FAST_TRACK_PRIORITY),
];

const CATALOG_REFRESH_TIERS: [(&str, i16); 4] = [
    (
        "star_viewer",
        CATALOG_REFRESH_VIEWER_PRIORITY + STAR_PRIORITY_BONUS,
    ),
    ("viewer", CATALOG_REFRESH_VIEWER_PRIORITY),
    (
        "star_background",
        CATALOG_REFRESH_BACKGROUND_PRIORITY + STAR_PRIORITY_BONUS,
    ),
    ("background", CATALOG_REFRESH_BACKGROUND_PRIORITY),
];

pub(crate) async fn record_queued_waits(pool: &PgPool) -> Result<(), sqlx::Error> {
    let wanted = wanted_waits();
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
        crate::metrics::record_star_priority_queued(kind.as_str(), tier, waiting_seconds);
    }
    Ok(())
}

fn wanted_waits() -> Vec<(JobKind, &'static str, i16)> {
    let tiered = TIERED_KINDS.iter().flat_map(|&(kind, base)| {
        Tier::ORDER.map(|tier| (kind, tier.label(), tier.job_priority(base)))
    });
    let catalog_refresh = CATALOG_REFRESH_TIERS
        .iter()
        .map(|&(label, priority)| (JobKind::CatalogRefresh, label, priority));
    tiered.chain(catalog_refresh).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_star_reader_refresh_has_its_own_priority_in_the_queue_metrics() {
        let refreshes: Vec<(&str, i16)> = wanted_waits()
            .into_iter()
            .filter(|(kind, _, _)| *kind == JobKind::CatalogRefresh)
            .map(|(_, tier, priority)| (tier, priority))
            .collect();

        assert_eq!(
            refreshes,
            [
                ("star_viewer", 20),
                ("viewer", 15),
                ("star_background", 10),
                ("background", 5)
            ]
        );
    }
}

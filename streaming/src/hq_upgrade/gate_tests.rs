use crate::db::postgres::test_nodes::core_and_api;
use crate::db::postgres::upgrade::UpgradeTier;
use crate::stream::quality;

use super::{pick, pick_on_core};

const TIERS: [UpgradeTier; 3] = [
    UpgradeTier::StarLiked,
    UpgradeTier::Liked,
    UpgradeTier::Rest,
];

const WAITING: &str = "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms,
                         storage_quality, hq_upgrade_pending, index_priority)
     SELECT 'gate-' || n, 'soundcloud:tracks:gate-' || n, 'Gate ' || n, 'gate ' || n, 1000,
            'sq', true, CASE WHEN n % 2 = 0 THEN 1 ELSE 5 END
     FROM generate_series(1, 8) AS n;";

fn waiting() -> Vec<String> {
    let mut urns: Vec<String> = (1..=8)
        .map(|n| format!("soundcloud:tracks:gate-{n}"))
        .collect();
    urns.sort();
    urns
}

fn sorted(mut urns: Vec<String>) -> Vec<String> {
    urns.sort();
    urns
}

#[tokio::test]
#[ignore = "requires a Postgres 17 server; builds its databases from api/migrations"]
async fn without_the_gate_two_nodes_download_every_track_twice() {
    let (core, api) = core_and_api(WAITING).await;

    let (on_core, on_api) = tokio::join!(
        pick(&core.pg, quality::SQ, &TIERS),
        pick(&api.pg, quality::SQ, &TIERS)
    );

    assert_eq!(sorted(on_core), waiting());
    assert_eq!(
        sorted(on_api),
        waiting(),
        "each node claims from its own copy, so SKIP LOCKED cannot keep them apart"
    );
    core.finish().await;
    api.finish().await;
}

#[tokio::test]
#[ignore = "requires a Postgres 17 server; builds its databases from api/migrations"]
async fn only_core_claims_even_where_the_switch_is_on_by_mistake() {
    let (core, api) = core_and_api(WAITING).await;

    let (on_core, on_api) = tokio::join!(
        pick_on_core(&core.pg, quality::SQ, &TIERS),
        pick_on_core(&api.pg, quality::SQ, &TIERS)
    );

    assert_eq!(sorted(on_core), waiting());
    assert_eq!(on_api, Vec::<String>::new());
    let claimed_on_core: i64 = core
        .value("SELECT count(*) FROM tracks WHERE hq_upgrade_attempts = 1")
        .await;
    let touched_on_api: i64 = api
        .value("SELECT count(*) FROM tracks WHERE hq_upgrade_attempts > 0 OR hq_upgrade_last_at IS NOT NULL")
        .await;
    assert_eq!(claimed_on_core, 8);
    assert_eq!(
        touched_on_api, 0,
        "the api node wrote tracks it does not own"
    );
    core.finish().await;
    api.finish().await;
}

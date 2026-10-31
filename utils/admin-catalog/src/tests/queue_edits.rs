use backend_contracts::admin_catalog::{AdminCommand, EnrichRetry, WantedLink, WantedStatus};
use sqlx::PgPool;
use uuid::Uuid;

use super::support::{sc_track_id, send, track};

async fn wanted(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO wanted_tracks (id, title, normalized_title, source, external_id, status)
         VALUES ($1, 'Night Drive', 'night drive', 'genius_crawl', '5521337', 'wanted')",
    )
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn wanted_state(pool: &PgPool, id: Uuid) -> (String, Option<Uuid>) {
    sqlx::query_as("SELECT status, track_id FROM wanted_tracks WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn a_wanted_track_links_to_the_indexed_track(pool: PgPool) {
    let wanted = wanted(&pool).await;
    let track = track(&pool, None).await;
    let sc = sc_track_id(&pool, track).await;

    let reply = send(
        &pool,
        AdminCommand::WantedLink {
            wanted_id: wanted,
            body: WantedLink { sc_track_id: sc },
        },
    )
    .await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.body["linked"], true);
    assert_eq!(wanted_state(&pool, wanted).await.1, Some(track));
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn linking_to_a_track_that_does_not_exist_leaves_the_wanted_track_alone(pool: PgPool) {
    let wanted = wanted(&pool).await;

    let reply = send(
        &pool,
        AdminCommand::WantedLink {
            wanted_id: wanted,
            body: WantedLink {
                sc_track_id: "999999999".to_owned(),
            },
        },
    )
    .await;

    assert_eq!(reply.status, 400);
    assert_eq!(
        wanted_state(&pool, wanted).await,
        ("wanted".to_owned(), None)
    );
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn a_wanted_status_is_checked_against_the_allowed_set(pool: PgPool) {
    let wanted = wanted(&pool).await;
    let set = |status: &str| AdminCommand::WantedSetStatus {
        wanted_id: wanted,
        body: WantedStatus {
            status: status.to_owned(),
        },
    };

    assert_eq!(send(&pool, set("skipped")).await.status, 200);
    assert_eq!(wanted_state(&pool, wanted).await.0, "skipped");
    assert_eq!(send(&pool, set("bogus")).await.status, 400);
    let missing = send(
        &pool,
        AdminCommand::WantedSetStatus {
            wanted_id: Uuid::new_v4(),
            body: WantedStatus {
                status: "skipped".to_owned(),
            },
        },
    )
    .await;
    assert_eq!(missing.status, 404);
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn abandoning_a_legacy_intent_wakes_the_playlist_once(pool: PgPool) {
    let urn = "soundcloud:playlists:77";
    sqlx::query(
        "INSERT INTO playlists (sc_playlist_id, title, title_normalized, urn) VALUES ('77', 'Mix', 'mix', $1)",
    )
    .bind(urn)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO playlist_membership_state (playlist_urn) VALUES ($1) ON CONFLICT DO NOTHING",
    )
    .bind(urn)
    .execute(&pool)
    .await
    .unwrap();
    let archive = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO playlist_legacy_membership_intents (archive_id, playlist_urn, source) VALUES ($1, $2, 'sync_queue')",
    )
    .bind(archive)
    .bind(urn)
    .execute(&pool)
    .await
    .unwrap();

    let reply = send(
        &pool,
        AdminCommand::LegacyPlaylistAbandon {
            archive_id: archive,
        },
    )
    .await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.body["playlistUrn"], urn);
    assert_eq!(reply.body["playlistUnblocked"], true);
    let (classification, woken): (String, bool) = sqlx::query_as(
        "SELECT i.classification, s.next_reconcile_at IS NOT NULL
         FROM playlist_legacy_membership_intents i
         JOIN playlist_membership_state s ON s.playlist_urn = i.playlist_urn
         WHERE i.archive_id = $1",
    )
    .bind(archive)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(classification, "abandoned");
    assert!(woken);

    let again = send(
        &pool,
        AdminCommand::LegacyPlaylistAbandon {
            archive_id: archive,
        },
    )
    .await;
    assert_eq!(again.status, 404);
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn an_enrich_retry_resets_one_track_or_every_failed_track(pool: PgPool) {
    let single = track(&pool, None).await;
    let failed = track(&pool, None).await;
    let healthy = track(&pool, None).await;
    for (track, state) in [(single, "done"), (failed, "dead"), (healthy, "done")] {
        sqlx::query("UPDATE tracks SET enrich_state = $1, enrich_attempts = 4 WHERE id = $2")
            .bind(state)
            .bind(track)
            .execute(&pool)
            .await
            .unwrap();
    }
    let state = |track| {
        let pool = pool.clone();
        async move {
            sqlx::query_as::<_, (Option<String>, i32)>(
                "SELECT enrich_state, enrich_attempts::int FROM tracks WHERE id = $1",
            )
            .bind(track)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };

    let sc = sc_track_id(&pool, single).await;
    let one = send(
        &pool,
        AdminCommand::EnrichRetry(EnrichRetry {
            sc_track_id: Some(sc),
            all_failed: false,
        }),
    )
    .await;
    assert_eq!(one.body["reset"], 1);
    assert_eq!(state(single).await, (Some("pending".to_owned()), 0));
    assert_eq!(state(failed).await.1, 4);

    let all = send(
        &pool,
        AdminCommand::EnrichRetry(EnrichRetry {
            sc_track_id: None,
            all_failed: true,
        }),
    )
    .await;
    assert_eq!(all.body["reset"], 1);
    assert_eq!(state(failed).await, (Some("pending".to_owned()), 0));
    assert_eq!(state(healthy).await, (Some("done".to_owned()), 4));
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn an_enrich_retry_refuses_a_malformed_track_id(pool: PgPool) {
    let reply = send(
        &pool,
        AdminCommand::EnrichRetry(EnrichRetry {
            sc_track_id: Some("not a track".to_owned()),
            all_failed: true,
        }),
    )
    .await;
    assert_eq!(reply.status, 400);
}

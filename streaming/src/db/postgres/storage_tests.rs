use uuid::Uuid;

use crate::db::postgres::test_nodes::core_and_api;

const URN: &str = "soundcloud:tracks:1874620031";

const STORED: &str = "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms,
                         storage_state, storage_quality, s3_verified_at)
     VALUES ('1874620031', 'soundcloud:tracks:1874620031', 'Night Drive', 'night drive', 213000,
             'ok', 'hq', '2026-10-01 10:00:00+00');
     INSERT INTO cdn_tracks (id, track_urn, quality, cdn_path, status)
     VALUES ('0192f0a4-7b3c-7d2e-9a41-5c6d7e8f9a0b', 'soundcloud:tracks:1874620031', 'single',
             'soundcloud_tracks_1874620031.m4a', 'ok');";

const OLD_CDN_ID: &str = "0192f0a4-7b3c-7d2e-9a41-5c6d7e8f9a0b";

#[tokio::test]
#[ignore = "requires a Postgres 17 server; builds its databases from api/migrations"]
async fn both_nodes_give_a_new_upload_the_same_id() {
    let (core, api) = core_and_api("").await;
    let urn = "soundcloud:tracks:1990011";

    let on_api = api
        .pg
        .insert_cdn_track(urn, "a.m4a", "pending")
        .await
        .unwrap();
    let on_core = core
        .pg
        .insert_cdn_track(urn, "a.m4a", "pending")
        .await
        .unwrap();
    let derived: Uuid = core
        .value("SELECT public.scd_uuid('cdn_tracks', 'soundcloud:tracks:1990011', 'single')")
        .await;

    assert_eq!(on_api, on_core, "two nodes made two rows for one upload");
    assert_eq!(on_api, derived.to_string());
    assert_eq!(derived.get_version_num(), 5);
    core.finish().await;
    api.finish().await;
}

#[tokio::test]
#[ignore = "requires a Postgres 17 server; builds its databases from api/migrations"]
async fn an_upload_again_updates_the_row_it_has_and_keeps_its_old_id() {
    let (core, api) = core_and_api(STORED).await;

    let again = api
        .pg
        .insert_cdn_track(URN, "again.m4a", "pending")
        .await
        .unwrap();
    api.pg.update_cdn_track_status(&again, "ok").await.unwrap();
    let cached = api.pg.find_cached_track(URN).await.unwrap();
    let rows: i64 = api.value("SELECT count(*) FROM cdn_tracks").await;

    assert_eq!(again, OLD_CDN_ID);
    assert_eq!(cached.map(|c| c.id).as_deref(), Some(OLD_CDN_ID));
    assert_eq!(rows, 1);
    core.finish().await;
    api.finish().await;
}

#[tokio::test]
#[ignore = "requires a Postgres 17 server; builds its databases from api/migrations"]
async fn a_lost_track_and_its_cdn_row_leave_the_api_node_in_one_transaction() {
    let (core, api) = core_and_api(STORED).await;

    let marked = api.pg.mark_storage_lost(OLD_CDN_ID, URN).await.unwrap();

    assert!(marked);
    let track: (String, Option<String>, bool, bool) = {
        let row = api
            .sql
            .query_one(
                "SELECT storage_state, storage_quality, s3_verified_at IS NULL,
                        s3_missing_at = updated_at
                 FROM tracks WHERE urn = $1",
                &[&URN],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1), row.get(2), row.get(3))
    };
    assert_eq!(track, ("pending".to_owned(), None, true, true));
    let shipped: (String, i64, i64, bool) = {
        let row = api
            .sql
            .query_one(
                "SELECT o.kind, jsonb_array_length(o.touched)::bigint,
                        (SELECT count(*) FROM cmd_pending p WHERE p.id = o.id),
                        o.xmin = (SELECT c.xmin FROM cdn_tracks c WHERE c.id = $1::text::uuid)
                 FROM cmd_outbox o",
                &[&OLD_CDN_ID],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1), row.get(2), row.get(3))
    };
    assert_eq!(
        shipped,
        ("track.storage_lost".to_owned(), 1, 1, true),
        "the command must ship in the transaction that marked the cdn row"
    );
    let status: String = api
        .value("SELECT status FROM cdn_tracks WHERE track_urn = 'soundcloud:tracks:1874620031'")
        .await;
    assert_eq!(status, "error");
    core.finish().await;
    api.finish().await;
}

#[tokio::test]
#[ignore = "requires a Postgres 17 server; builds its databases from api/migrations"]
async fn on_core_a_lost_track_is_marked_in_place() {
    let (core, api) = core_and_api(STORED).await;

    assert!(core.pg.mark_storage_lost(OLD_CDN_ID, URN).await.unwrap());
    assert!(!core.pg.mark_storage_lost(OLD_CDN_ID, URN).await.unwrap());

    let state: String = core
        .value("SELECT storage_state FROM tracks WHERE urn = 'soundcloud:tracks:1874620031'")
        .await;
    let outbox: i64 = core.value("SELECT count(*) FROM cmd_outbox").await;
    assert_eq!(state, "pending");
    assert_eq!(outbox, 0, "core runs the kind itself and ships nothing");
    core.finish().await;
    api.finish().await;
}

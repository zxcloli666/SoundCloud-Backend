use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use uuid::Uuid;
use write_through as wt;
use wt::{Mode, ObservePlaylists, ObserveProfile, ObserveTracks, ObserveUsers};

use crate::harness::Pair;

const PROD_TRACK_ID: &str = "6f1c2b9e-4d7a-4c3e-9b8f-2a5d7e1c0b94";
const PROD_TRACK: &str = "INSERT INTO public.tracks (
        id, sc_track_id, urn, title, title_normalized, duration_ms, sharing, artwork_url, permalink_url,
        uploader_sc_user_id, sc_created_at, sc_last_modified, sc_synced_at, sc_observation,
        index_priority, storage_priority, storage_state, index_state, created_at, updated_at)
    VALUES ('6f1c2b9e-4d7a-4c3e-9b8f-2a5d7e1c0b94', '1874620031', 'soundcloud:tracks:1874620031',
        'Night Drive', 'night drive', 213000, 'public',
        'https://i1.sndcdn.com/artworks-1874620031-t500x500.jpg', 'https://soundcloud.com/kavinsky-fan/night-drive',
        '204411307', '2026-01-15 12:00:00+00', '2026-09-01 10:00:00+00', '2026-10-28 08:00:00+00', 517000,
        5, 5, 'ok', 'indexed', '2026-01-15 12:00:05+00', '2026-10-28 08:00:00+00')";
const PROD_USER: &str = "INSERT INTO public.users (
        sc_user_id, urn, username, username_normalized, full_name, city, avatar_url, verified,
        followers_count, sc_last_modified, sc_synced_at, sc_observation, created_at, updated_at)
    VALUES ('204411307', 'soundcloud:users:204411307', 'kavinsky-fan', 'kavinsky fan', 'Night Rider', 'Paris',
        'https://i1.sndcdn.com/avatars-204411307-large.jpg', false, 812,
        '2026-08-01 09:00:00+00', '2026-10-28 08:00:00+00', 517001, '2025-11-02 18:00:00+00', '2026-10-28 08:00:00+00')";
const PROD_PLAYLIST: &str = "INSERT INTO public.playlists (
        urn, sc_playlist_id, title, title_normalized, track_count, sharing, owner_sc_user_id,
        sc_last_modified, sc_synced_at, sc_observation, created_at, updated_at)
    VALUES ('soundcloud:playlists:1990011', '1990011', 'Drive Mix', 'drive mix', 12, 'public', '204411307',
        '2026-08-10 09:00:00+00', '2026-10-28 08:00:00+00', 517002, '2026-02-01 10:00:00+00', '2026-10-28 08:00:00+00');
    INSERT INTO public.playlist_membership_state (playlist_urn, projection_track_count, sync_status, next_reconcile_at, updated_at)
    VALUES ('soundcloud:playlists:1990011', 12, 'clean', '2026-11-01 00:00:00+00', '2026-10-28 08:00:00+00')";
const HLC_1: i64 = 28_693_349_570_866_705;
const HLC_2: i64 = 28_693_349_670_866_705;
const CONFIRM_TRACK: &str =
    include_str!("../../../../jobs/queries/sync_queue/actions/confirm_track_update.sql");
const EDIT: &str = "Night Drive (Edit)";

fn at(stamp: &str) -> DateTime<Utc> {
    stamp.parse().unwrap()
}

fn track(sc_id: &str, title: &str, last_modified: &str, priority: i16) -> Value {
    json!({
        "sc_track_id": sc_id,
        "urn": format!("soundcloud:tracks:{sc_id}"),
        "title": title,
        "title_normalized": title.to_lowercase(),
        "description": "night ride",
        "genre": "Synthwave",
        "tags": ["synthwave", "night drive"],
        "duration_ms": 213000,
        "artwork_url": format!("https://i1.sndcdn.com/artworks-{sc_id}-t500x500.jpg"),
        "permalink_url": "https://soundcloud.com/kavinsky-fan/night-drive",
        "waveform_url": format!("https://wave.sndcdn.com/{sc_id}_m.json"),
        "language": null,
        "isrc": null,
        "metadata_artist": null,
        "sharing": "public",
        "sc_created_at": "2026-01-15T12:00:00Z",
        "sc_last_modified": last_modified,
        "release_year": 2026,
        "release_date": "2026-01-15",
        "uploader_sc_user_id": "204411307",
        "uploader_urn": "soundcloud:users:204411307",
        "uploader_username": "kavinsky-fan",
        "uploader_avatar_url": "https://i1.sndcdn.com/avatars-204411307-large.jpg",
        "play_count_sc": 12004,
        "likes_count_sc": 431,
        "reposts_count_sc": 17,
        "comments_count_sc": 9,
        "needs_duration_resolve": false,
        "is_cover": false,
        "sc_metadata": {"kind": "track", "policy": "ALLOW"},
        "index_priority": priority,
        "storage_priority": priority,
    })
}

fn user(username: &str, keys: &[&str]) -> Value {
    json!({
        "sc_user_id": "204411307",
        "urn": "soundcloud:users:204411307",
        "username": username,
        "username_normalized": username.replace('-', " "),
        "full_name": null,
        "first_name": null,
        "last_name": null,
        "permalink": username,
        "permalink_url": format!("https://soundcloud.com/{username}"),
        "avatar_url": "https://i1.sndcdn.com/avatars-204411307-t500x500.jpg",
        "country": null,
        "city": null,
        "description": null,
        "verified": false,
        "followers_count": 815,
        "followings_count": null,
        "tracks_count": 44,
        "playlists_count": null,
        "reposts_count": null,
        "comments_count": null,
        "kind": "user",
        "sc_created_at": "2025-11-02T18:00:00Z",
        "sc_last_modified": "2026-10-29T09:00:00Z",
        "keys": keys,
    })
}

fn playlist(track_count: i32) -> Value {
    json!({
        "urn": "soundcloud:playlists:1990011",
        "sc_playlist_id": "1990011",
        "title": "Drive Mix",
        "title_normalized": "drive mix",
        "description": null,
        "genre": "Synthwave",
        "tags": ["drive"],
        "artwork_url": null,
        "permalink_url": "https://soundcloud.com/kavinsky-fan/sets/drive-mix",
        "owner_sc_user_id": "204411307",
        "owner_urn": "soundcloud:users:204411307",
        "owner_username": "kavinsky-fan",
        "track_count": track_count,
        "duration_ms": 2_400_000,
        "playlist_type": "playlist",
        "kind": "playlist",
        "sharing": "public",
        "release_year": null,
        "release_date": null,
        "label_name": null,
        "likes_count_sc": 31,
        "reposts_count_sc": 2,
        "sc_created_at": "2026-02-01T10:00:00Z",
        "sc_last_modified": "2026-10-29T10:00:00Z",
        "sc_metadata": {"set_type": "playlist"},
    })
}

fn tracks(mode: Mode, observation: i64, stamp: &str, rows: Vec<Value>) -> ObserveTracks {
    ObserveTracks {
        mode,
        observed_at: at(stamp),
        observation,
        rows,
    }
}

async fn scalar<T>(pool: &sqlx::PgPool, sql: &str) -> T
where
    T: for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres> + Send + Unpin,
{
    sqlx::query_scalar(sql)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

#[tokio::test]
async fn a_new_track_gets_the_same_v5_id_on_api_and_core() {
    let pair = Pair::new("").await;
    let upsert = tracks(
        Mode::Upsert,
        HLC_1,
        "2026-10-30T10:00:00.123456Z",
        vec![track("1874620099", "Sunset", "2026-10-29T10:00:00Z", 1)],
    );
    let output = pair
        .run(&upsert, &["lyrics_lookup_state"])
        .await
        .assert_same();
    let v5: Uuid = scalar(
        &pair.core,
        "SELECT public.scd_uuid('tracks', 'soundcloud:tracks:1874620099')",
    )
    .await;
    assert_eq!(
        output,
        json!([{"sc_track_id": "1874620099", "id": v5, "was_new": true, "applied": true}])
    );
    assert_eq!(v5.get_version_num(), 5);
    let stamps: bool = scalar(
        &pair.core,
        "SELECT created_at = '2026-10-30T10:00:00.123456Z' AND updated_at = created_at
                AND sc_synced_at = created_at AND enrich_next_run_at = created_at
                AND sc_observation = 28693349570866705 AND pipeline_held = false
         FROM public.tracks",
    )
    .await;
    assert!(stamps);
    pair.finish().await;
}

#[tokio::test]
async fn a_changed_prod_track_keeps_its_v4_id() {
    let pair = Pair::new(PROD_TRACK).await;
    let upsert = tracks(
        Mode::Upsert,
        HLC_1,
        "2026-10-30T10:00:00Z",
        vec![track(
            "1874620031",
            "Night Drive (Remastered)",
            "2026-10-29T10:00:00Z",
            1,
        )],
    );
    let output = pair
        .run(&upsert, &["lyrics_lookup_state"])
        .await
        .assert_same();
    assert_eq!(
        output,
        json!([{"sc_track_id": "1874620031", "id": PROD_TRACK_ID, "was_new": false, "applied": true}])
    );
    let row: (Uuid, String, i64, i16) =
        sqlx::query_as("SELECT id, title, sc_observation, index_priority FROM public.tracks")
            .fetch_one(&pair.api)
            .await
            .unwrap();
    assert_eq!(
        row,
        (
            PROD_TRACK_ID.parse().unwrap(),
            "Night Drive (Remastered)".to_owned(),
            HLC_1,
            1
        )
    );
    pair.finish().await;
}

#[tokio::test]
async fn an_unchanged_track_only_lifts_its_priority() {
    let same = track("1874620031", "Night Drive", "2026-09-01T10:00:00Z", 2);
    let pair = Pair::new(PROD_TRACK).await;
    let upsert = tracks(Mode::Upsert, HLC_1, "2026-10-30T10:00:00Z", vec![same]);
    let output = pair
        .run(&upsert, &["lyrics_lookup_state"])
        .await
        .assert_same();
    assert_eq!(
        output,
        json!([{"sc_track_id": "1874620031", "id": PROD_TRACK_ID, "was_new": false, "applied": false}])
    );
    let row: (String, i16, i16, i64) = sqlx::query_as(
        "SELECT title, index_priority, storage_priority, sc_observation FROM public.tracks",
    )
    .fetch_one(&pair.core)
    .await
    .unwrap();
    assert_eq!(row, ("Night Drive".to_owned(), 2, 2, 517000));
    pair.finish().await;
}

#[tokio::test]
async fn absent_tracks_insert_only_the_missing_rows() {
    let pair = Pair::new(PROD_TRACK).await;
    let absent = tracks(
        Mode::Absent,
        0,
        "2026-10-30T10:00:00Z",
        vec![
            track("1874620031", "Overwritten?", "2026-10-29T10:00:00Z", 5),
            track("1874620100", "Fresh One", "2026-10-29T10:00:00Z", 5),
        ],
    );
    let output = pair
        .run(&absent, &["lyrics_lookup_state"])
        .await
        .assert_same();
    let v5: Uuid = scalar(
        &pair.core,
        "SELECT public.scd_uuid('tracks', 'soundcloud:tracks:1874620100')",
    )
    .await;
    assert_eq!(
        output,
        json!([{"sc_track_id": "1874620100", "id": v5, "was_new": true, "applied": true}])
    );
    let kept: String = scalar(
        &pair.core,
        "SELECT title FROM public.tracks WHERE sc_track_id = '1874620031'",
    )
    .await;
    assert_eq!(kept, "Night Drive");
    pair.finish().await;
}

#[tokio::test]
async fn a_bump_lifts_priority_and_releases_the_hold() {
    let pair = Pair::new(&format!(
        "{PROD_TRACK}; UPDATE public.tracks SET pipeline_held = true"
    ))
    .await;
    let bump = tracks(
        Mode::Bump,
        0,
        "2026-10-30T10:00:00Z",
        vec![json!({"sc_track_id": "1874620031", "index_priority": 1, "storage_priority": 1})],
    );
    let output = pair
        .run(&bump, &["lyrics_lookup_state"])
        .await
        .assert_same();
    assert_eq!(output, json!([]));
    let row: (i16, i16, bool) =
        sqlx::query_as("SELECT index_priority, storage_priority, pipeline_held FROM public.tracks")
            .fetch_one(&pair.core)
            .await
            .unwrap();
    assert_eq!(row, (1, 1, false));
    pair.finish().await;
}

#[tokio::test]
async fn a_backlog_observation_never_overwrites_newer_core_data() {
    let pair = Pair::new(PROD_TRACK).await;
    let newer = tracks(
        Mode::Upsert,
        HLC_2,
        "2026-10-30T10:05:00Z",
        vec![track(
            "1874620031",
            "Night Drive (Final)",
            "2026-10-29T12:00:00Z",
            5,
        )],
    );
    let mut core = pair.core.begin().await.unwrap();
    wt::call(&mut core, &newer).await.unwrap();
    core.commit().await.unwrap();
    let backlog = tracks(
        Mode::Upsert,
        HLC_1,
        "2026-10-30T10:06:00Z",
        vec![track(
            "1874620031",
            "Night Drive (Draft)",
            "2026-10-29T11:00:00Z",
            5,
        )],
    );
    let run = pair.run(&backlog, &["lyrics_lookup_state"]).await;
    assert_eq!(run.api[0]["applied"], json!(true));
    assert_eq!(run.core[0]["applied"], json!(false));
    assert_eq!(
        run.diverged[1..],
        [
            "api and core differ: tracks",
            "dispatch on core: Verdict { status: Some(\"ok\"), reasserted: 1, purged: 0, dead: 0 }",
        ]
    );
    let core_row: (String, i64, DateTime<Utc>) =
        sqlx::query_as("SELECT title, sc_observation, sc_synced_at FROM public.tracks")
            .fetch_one(&pair.core)
            .await
            .unwrap();
    assert_eq!(
        core_row,
        (
            "Night Drive (Final)".to_owned(),
            HLC_2,
            at("2026-10-30T10:05:00Z")
        )
    );
    pair.finish().await;
}

fn confirmed_edit() -> String {
    format!(
        "{PROD_TRACK};
         UPDATE public.tracks SET title = '{EDIT}', title_normalized = lower('{EDIT}'),
             sc_desired = jsonb_build_object('title', '{EDIT}'),
             sc_observation = {HLC_1}, sc_mutation_observation = {HLC_1}, sc_write_confirmed = false;
         PREPARE confirm(text, jsonb) AS {CONFIRM_TRACK};
         EXECUTE confirm('1874620031', jsonb_build_object('title', '{EDIT}'));
         DEALLOCATE confirm"
    )
}

async fn confirm_and_skew(pair: &Pair) -> (i64, i64) {
    let confirmed = scalar(
        &pair.core,
        "SELECT sc_mutation_observation FROM public.tracks",
    )
    .await;
    let skew = scalar(&pair.core, "SELECT public.scd_observation_skew()").await;
    (confirmed, skew)
}

fn observed_edit(observation: i64) -> ObserveTracks {
    tracks(
        Mode::Upsert,
        observation,
        "2026-10-30T10:00:00Z",
        vec![track("1874620031", EDIT, "2026-10-29T12:00:00Z", 5)],
    )
}

#[tokio::test]
async fn an_api_observation_up_to_the_skew_ahead_of_a_core_confirm_is_refused() {
    let pair = Pair::new(&confirmed_edit()).await;
    let (confirmed, skew) = confirm_and_skew(&pair).await;
    assert!(confirmed > HLC_1);
    assert_eq!(skew, 1_000_000 << 4);
    for ahead in [1, skew / 2, skew] {
        let output = pair
            .run(&observed_edit(confirmed + ahead), &["lyrics_lookup_state"])
            .await
            .assert_same();
        assert_eq!(output[0]["applied"], json!(false), "{ahead} ahead");
    }
    let row: (i64, bool, Value) =
        sqlx::query_as("SELECT sc_observation, sc_write_confirmed, sc_desired FROM public.tracks")
            .fetch_one(&pair.core)
            .await
            .unwrap();
    assert_eq!(row, (confirmed, true, json!({"title": EDIT})));
    pair.finish().await;
}

#[tokio::test]
async fn an_observation_more_than_the_skew_after_a_confirm_is_accepted() {
    let pair = Pair::new(&confirmed_edit()).await;
    let (confirmed, skew) = confirm_and_skew(&pair).await;
    let output = pair
        .run(
            &observed_edit(confirmed + skew + 1),
            &["lyrics_lookup_state"],
        )
        .await
        .assert_same();
    assert_eq!(output[0]["applied"], json!(true));
    let row: (i64, i64, bool, Value) = sqlx::query_as(
        "SELECT sc_observation, sc_mutation_observation, sc_write_confirmed, sc_desired FROM public.tracks",
    )
    .fetch_one(&pair.core)
    .await
    .unwrap();
    assert_eq!(row, (confirmed + skew + 1, confirmed, false, json!({})));
    pair.finish().await;
}

#[tokio::test]
async fn a_thin_user_payload_keeps_the_fields_it_does_not_carry() {
    let pair = Pair::new(PROD_USER).await;
    let upsert = ObserveUsers {
        mode: Mode::Upsert,
        observed_at: at("2026-10-30T10:00:00Z"),
        observation: HLC_1,
        rows: vec![user("kavinsky-fan2", &["avatar_url", "permalink", "kind"])],
    };
    let output = pair.run(&upsert, &[]).await.assert_same();
    assert_eq!(
        output,
        json!([{"sc_user_id": "204411307", "was_new": false}])
    );
    let row: (String, Option<String>, Option<String>, Option<String>, i64) = sqlx::query_as(
        "SELECT username, full_name, city, permalink, followers_count FROM public.users",
    )
    .fetch_one(&pair.core)
    .await
    .unwrap();
    assert_eq!(
        row,
        (
            "kavinsky-fan2".to_owned(),
            Some("Night Rider".to_owned()),
            Some("Paris".to_owned()),
            Some("kavinsky-fan2".to_owned()),
            815
        )
    );
    pair.finish().await;
}

#[tokio::test]
async fn absent_users_skip_the_ones_already_stored() {
    let pair = Pair::new(PROD_USER).await;
    let mut other = user("drive-club", &[]);
    other["sc_user_id"] = json!("1500300");
    other["urn"] = json!("soundcloud:users:1500300");
    let absent = ObserveUsers {
        mode: Mode::Absent,
        observed_at: at("2026-10-30T10:00:00Z"),
        observation: 0,
        rows: vec![user("ignored", &[]), other],
    };
    let output = pair.run(&absent, &[]).await.assert_same();
    assert_eq!(output, json!([{"sc_user_id": "1500300", "was_new": true}]));
    pair.finish().await;
}

#[tokio::test]
async fn a_new_playlist_gets_its_membership_state() {
    let pair = Pair::new("").await;
    let upsert = ObservePlaylists {
        mode: Mode::Upsert,
        observed_at: at("2026-10-30T10:00:00Z"),
        observation: HLC_1,
        rows: vec![playlist(12)],
    };
    let output = pair.run(&upsert, &[]).await.assert_same();
    assert_eq!(
        output,
        json!([{"urn": "soundcloud:playlists:1990011", "was_new": true}])
    );
    let state: (String, i32, DateTime<Utc>) = sqlx::query_as(
        "SELECT sync_status, projection_track_count, next_reconcile_at FROM public.playlist_membership_state",
    )
    .fetch_one(&pair.core)
    .await
    .unwrap();
    assert_eq!(
        state,
        ("unhydrated".to_owned(), 0, at("2026-10-30T10:00:00Z"))
    );
    pair.finish().await;
}

#[tokio::test]
async fn a_track_count_change_wakes_the_membership_reconcile() {
    let pair = Pair::new(PROD_PLAYLIST).await;
    let upsert = ObservePlaylists {
        mode: Mode::Upsert,
        observed_at: at("2026-10-30T10:00:00Z"),
        observation: HLC_1,
        rows: vec![playlist(13)],
    };
    let output = pair.run(&upsert, &[]).await.assert_same();
    assert_eq!(
        output,
        json!([{"urn": "soundcloud:playlists:1990011", "was_new": false}])
    );
    let state: (DateTime<Utc>, DateTime<Utc>) = sqlx::query_as(
        "SELECT next_reconcile_at, updated_at FROM public.playlist_membership_state",
    )
    .fetch_one(&pair.core)
    .await
    .unwrap();
    assert_eq!(
        state,
        (at("2026-10-30T10:00:00Z"), at("2026-10-30T10:00:00Z"))
    );
    pair.finish().await;
}

#[tokio::test]
async fn absent_playlists_leave_stored_ones_alone() {
    let pair = Pair::new(PROD_PLAYLIST).await;
    let mut other = playlist(3);
    other["urn"] = json!("soundcloud:playlists:1990012");
    other["sc_playlist_id"] = json!("1990012");
    let absent = ObservePlaylists {
        mode: Mode::Absent,
        observed_at: at("2026-10-30T10:00:00Z"),
        observation: 0,
        rows: vec![playlist(99), other],
    };
    let output = pair.run(&absent, &[]).await.assert_same();
    assert_eq!(
        output,
        json!([{"urn": "soundcloud:playlists:1990012", "was_new": true}])
    );
    let kept: i32 = scalar(
        &pair.core,
        "SELECT track_count FROM public.playlists WHERE urn = 'soundcloud:playlists:1990011'",
    )
    .await;
    assert_eq!(kept, 12);
    pair.finish().await;
}

#[tokio::test]
async fn a_newer_profile_replaces_the_stored_one() {
    let pair = Pair::new(&format!(
        "{PROD_USER}; INSERT INTO public.user_profiles (soundcloud_user_id, profile_json, synced_at, sc_observation)
         VALUES ('204411307', '{{\"urn\": \"soundcloud:users:204411307\", \"username\": \"kavinsky-fan\"}}', '2026-10-28 08:00:00+00', 517001)"
    ))
    .await;
    let profile = ObserveProfile {
        user_id: "204411307".to_owned(),
        profile: json!({"urn": "soundcloud:users:204411307", "username": "kavinsky-fan2", "plan": "Free"}),
        observed_at: at("2026-10-30T10:00:00Z"),
        observation: HLC_1,
    };
    let output = pair.run(&profile, &[]).await.assert_same();
    assert_eq!(output, Value::Null);
    let stored: (Value, i64, DateTime<Utc>) =
        sqlx::query_as("SELECT profile_json, sc_observation, synced_at FROM public.user_profiles")
            .fetch_one(&pair.core)
            .await
            .unwrap();
    assert_eq!(
        stored,
        (profile.profile.clone(), HLC_1, at("2026-10-30T10:00:00Z"))
    );
    pair.finish().await;
}

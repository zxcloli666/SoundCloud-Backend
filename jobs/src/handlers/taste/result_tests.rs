use std::collections::{BTreeMap, HashMap};

use backend_contracts::pipeline::{Producer, TasteBaselines, TasteMetrics};
use sqlx::PgPool;

use super::super::dataset::EventRow;
use super::super::history::EventKind;
use super::super::pooling::Pooling;
use super::*;

const VERSION: &str = "taste-202609241200-0a1b2c3d";

fn result(status: WorkerStatus, reason: Option<WorkerReason>) -> TasteResult {
    let trained = status == WorkerStatus::Ok;
    TasteResult {
        input_object: "taste-input-1".to_owned(),
        status,
        reason,
        detail: None,
        producer: Producer {
            worker_id: "gpu-1".to_owned(),
            build: "test".to_owned(),
            models: BTreeMap::new(),
            sync_version: None,
        },
        version: trained.then(|| VERSION.to_owned()),
        object: trained.then(|| VERSION.to_owned()),
        dim: TRACKS_TASTE_DIMENSIONS,
        items_count: trained.then_some(3),
        users_count: trained.then_some(2),
        metrics: trained.then_some(TasteMetrics {
            recall_at_50: 0.3,
            ndcg_at_20: 0.2,
            cold_recall_at_50: 0.1,
            coverage_at_50: 0.4,
            baselines: TasteBaselines {
                popularity: 0.1,
                item2vec: 0.2,
                content: 0.15,
            },
        }),
    }
}

#[test]
fn a_trained_result_names_the_version_it_serves() {
    assert_eq!(
        judge(&result(WorkerStatus::Ok, None)).ok(),
        Some(Verdict::Trained {
            version: VERSION.to_owned(),
            items: 3
        })
    );
}

#[test]
fn a_model_below_its_baselines_keeps_the_serving_version() {
    assert_eq!(
        judge(&result(
            WorkerStatus::Rejected,
            Some(WorkerReason::BelowBaseline)
        ))
        .ok(),
        Some(Verdict::Untrained)
    );
    assert_eq!(
        judge(&result(
            WorkerStatus::Empty,
            Some(WorkerReason::TooFewUsers)
        ))
        .ok(),
        Some(Verdict::Untrained)
    );
}

#[test]
fn a_lost_training_asks_for_a_fresh_export() {
    assert_eq!(
        judge(&result(
            WorkerStatus::Failed,
            Some(WorkerReason::WorkerLost)
        ))
        .ok(),
        Some(Verdict::Reopen)
    );
}

#[test]
fn a_result_that_breaks_the_contract_is_refused() {
    let mut unnamed = result(WorkerStatus::Ok, None);
    unnamed.version = None;
    let mut elsewhere = result(WorkerStatus::Ok, None);
    elsewhere.object = Some("taste-202609241200-ffffffff".to_owned());
    let mut bad_pattern = result(WorkerStatus::Ok, None);
    bad_pattern.version = Some("latest".to_owned());
    bad_pattern.object = Some("latest".to_owned());
    let mut wide = result(WorkerStatus::Ok, None);
    wide.dim = 256;

    for broken in [
        unnamed,
        elsewhere,
        bad_pattern,
        wide,
        result(WorkerStatus::Ok, Some(WorkerReason::BelowBaseline)),
        result(WorkerStatus::Rejected, None),
        result(WorkerStatus::Empty, Some(WorkerReason::EmptyVocab)),
        result(WorkerStatus::Rejected, Some(WorkerReason::TooFewUsers)),
    ] {
        assert!(judge(&broken).is_err(), "{broken:?} was accepted");
    }
}

#[test]
fn every_version_gets_a_collection_of_its_own_behind_the_alias() {
    let name = collection_name(VERSION);

    assert_eq!(name, "tracks_taste_202609241200_0a1b2c3d");
    assert!(name.starts_with(TRACKS_TASTE.prefix));
    assert_ne!(name, TRACKS_TASTE.alias);
}

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO user_events (sc_user_id, sc_track_id, event_type, weight, created_at) VALUES
            ('soundcloud:users:7', 'soundcloud:tracks:101', 'like', 1.0, now() - interval '3 days'),
            ('7', '102', 'like', 1.0, now() - interval '3 days'),
            ('7', '103', 'playlist_add', 0.9, now() - interval '2 days'),
            ('7', '104', 'full_play', 0.3, now() - interval '2 days'),
            ('7', '104', 'full_play', 0.3, now() - interval '1 day'),
            ('7', '105', 'skip', 0.0, now() - interval '1 day'),
            ('7', '106', 'skip', -0.8, now() - interval '1 day'),
            ('7', '107', 'like', 1.0, now() - interval '400 days'),
            ('7', '108', 'dislike', -1.0, now() - interval '1 day'),
            ('8', '101', 'full_play', 0.3, now() - interval '1 day'),
            ('9', 'not-a-track', 'like', 1.0, now() - interval '1 day');
         INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state) VALUES
            ('7', '101', true),
            ('7', '102', false),
            ('7', '109', true),
            ('soundcloud:users:10', '110', true);
         INSERT INTO disliked_tracks (sc_user_id, sc_track_id, created_at) VALUES
            ('soundcloud:users:7', '111', now() - interval '1 day');",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn exported(pool: &PgPool) -> anyhow::Result<Vec<(String, i64, EventKind, bool)>> {
    let rows = sqlx::query_file_as!(EventRow, "queries/taste/export_events.sql", 180)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let timed = row.unix_s.is_some();
            let (user, event) = row.into_event()?;
            Some((user, i64::try_from(event.track).ok()?, event.kind, timed))
        })
        .collect())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn the_export_follows_the_signal_rules_of_the_design(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool).await?;

    let rows = exported(&pool).await?;
    let seven: Vec<(i64, EventKind, bool)> = rows
        .iter()
        .filter(|(user, ..)| user == "7")
        .map(|(_, track, kind, timed)| (*track, *kind, *timed))
        .collect();

    assert!(seven.contains(&(101, EventKind::Like, true)));
    assert!(
        !seven.contains(&(101, EventKind::LikeImport, false)),
        "an imported like beside its own UI like is the same signal twice"
    );
    assert!(
        !seven.iter().any(|(track, ..)| *track == 102),
        "a withdrawn like is not taste"
    );
    assert!(seven.contains(&(103, EventKind::PlaylistAdd, true)));
    assert_eq!(
        seven
            .iter()
            .filter(|row| **row == (104, EventKind::FullPlay, true))
            .count(),
        2
    );
    assert!(
        !seven.iter().any(|(track, ..)| *track == 105),
        "a skip after most of the track is neutral"
    );
    assert!(seven.contains(&(106, EventKind::Skip, true)));
    assert!(
        !seven.iter().any(|(track, ..)| *track == 107),
        "events older than the history window stay out"
    );
    assert!(
        !seven.iter().any(|(track, ..)| *track == 108),
        "a dislike event outlives its undo, only disliked_tracks counts"
    );
    assert!(seven.contains(&(109, EventKind::LikeImport, false)));
    assert!(seven.contains(&(111, EventKind::Dislike, true)));
    assert_eq!(
        seven.first().map(|(_, kind, _)| *kind),
        Some(EventKind::LikeImport),
        "events without a time come first"
    );
    assert!(rows.iter().any(|(user, track, kind, _)| user == "10"
        && *track == 110
        && *kind == EventKind::LikeImport));
    assert!(!rows.iter().any(|(user, ..)| user == "9"));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn an_imported_like_with_a_soundcloud_like_time_is_a_timed_like(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO user_events (sc_user_id, sc_track_id, event_type, weight, created_at) VALUES
            ('7', '104', 'like', 1.0, now() - interval '1 day');
         INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state, created_at, liked_at) VALUES
            ('7', '101', true, now(), now() - interval '10 days'),
            ('7', '102', true, now(), now() - interval '400 days'),
            ('7', '103', true, now(), NULL),
            ('7', '104', true, now(), now() - interval '20 days'),
            ('7', '105', false, now(), now() - interval '5 days'),
            ('7', '106', true, now(), now() + interval '10 days');",
    )
    .execute(&pool)
    .await?;
    let rows = sqlx::query_file_as!(EventRow, "queries/taste/export_events.sql", 180)
        .fetch_all(&pool)
        .await?;
    let seven: Vec<(i64, EventKind, Option<i64>)> = rows
        .into_iter()
        .filter_map(|row| {
            let unix_s = row.unix_s;
            let (user, event) = row.into_event()?;
            (user == "7").then_some((i64::try_from(event.track).ok()?, event.kind, unix_s))
        })
        .collect();
    let now = chrono::Utc::now().timestamp();
    let liked = seven
        .iter()
        .find(|(track, ..)| *track == 101)
        .ok_or_else(|| anyhow::anyhow!("the timed import is missing"))?;
    assert_eq!(liked.1, EventKind::Like);
    assert!(
        liked
            .2
            .is_some_and(|unix_s| (now - 10 * 86_400 - unix_s).abs() < 60),
        "the like carries the SoundCloud like time, not the snapshot time"
    );
    assert!(seven.contains(&(102, EventKind::LikeImport, None)));
    assert!(seven.contains(&(103, EventKind::LikeImport, None)));
    assert!(seven.contains(&(106, EventKind::LikeImport, None)));
    assert_eq!(
        seven.iter().filter(|(track, ..)| *track == 104).count(),
        1,
        "a like made in the app is not repeated by its imported copy"
    );
    assert!(!seven.iter().any(|(track, ..)| *track == 105));

    let pooled = sqlx::query_file_as!(
        EventRow,
        "queries/taste/user_events.sql",
        180,
        &["7".to_owned()]
    )
    .fetch_all(&pool)
    .await?;
    assert!(pooled.into_iter().any(|row| {
        let unix_s = row.unix_s;
        row.into_event().is_some_and(|(_, event)| {
            event.track == 101 && event.kind == EventKind::Like && unix_s.is_some()
        })
    }));
    Ok(())
}

fn pooling() -> Pooling {
    Pooling::from_json(&serde_json::json!({
        "w": {"like": 1.0, "like_import": 1.0, "playlist_add": 1.0, "full_play": 0.5, "skip": -0.5, "dislike": -1.0},
        "tau_days": 30.0
    }))
    .unwrap_or_else(|error| panic!("{error}"))
}

fn axis(index: usize) -> Vec<f32> {
    let mut vector = vec![0.0; 128];
    if let Some(slot) = vector.get_mut(index) {
        *slot = 1.0;
    }
    vector
}

#[sqlx::test(migrations = "../api/migrations")]
async fn only_listeners_with_a_positive_get_a_vector(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool).await?;
    let items: HashMap<u64, Vec<f32>> = (101..=111)
        .map(|track| (track, axis(track as usize - 100)))
        .collect();

    let vectors = vectors::pool_everyone(&pool, 180, &pooling(), &items, Utc::now().timestamp())
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let users: Vec<&str> = vectors.iter().map(|(user, _)| user.as_str()).collect();

    assert_eq!(users, vec!["10", "7"]);
    for (_, vector) in &vectors {
        let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4);
    }
    Ok(())
}

async fn record(
    pool: &PgPool,
    version: &str,
    input: &str,
    trained_days_ago: i32,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO taste_model_versions
             (version, input_object, collection, trained_at, dim, pooling, metrics, items_count, users_count)
         VALUES ($1, $2, $3, now() - $4::int * interval '1 day', 128, '{}', '{}', 1, 1)",
    )
    .bind(version)
    .bind(input)
    .bind(collection_name(version))
    .bind(trained_days_ago)
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn activating_a_version_retires_the_previous_one_and_resets_the_refresh_mark(
    pool: PgPool,
) -> anyhow::Result<()> {
    let older = "taste-202609201200-00000001";
    let newer = "taste-202609241200-00000002";
    record(&pool, older, "taste-input-a", 4).await?;
    record(&pool, newer, "taste-input-b", 0).await?;
    let computed_at = Utc::now();

    let first = activate(&pool, older, trained(&pool, older).await?, computed_at)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let second = activate(&pool, newer, trained(&pool, newer).await?, computed_at)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    let applied: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM taste_model_versions WHERE applied_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await?;

    assert!(first && second);
    assert_eq!(active(&pool).await?, vec![newer.to_owned()]);
    assert_eq!(applied, 2);
    assert_eq!(
        mark(&pool, newer).await?,
        Some(computed_at.timestamp_micros())
    );
    Ok(())
}

async fn trained(pool: &PgPool, version: &str) -> anyhow::Result<DateTime<Utc>> {
    Ok(
        sqlx::query_scalar("SELECT trained_at FROM taste_model_versions WHERE version = $1")
            .bind(version)
            .fetch_one(pool)
            .await?,
    )
}

async fn active(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(
        sqlx::query_scalar("SELECT version FROM taste_model_versions WHERE active")
            .fetch_all(pool)
            .await?,
    )
}

async fn mark(pool: &PgPool, version: &str) -> anyhow::Result<Option<i64>> {
    let mark: Option<chrono::NaiveDateTime> =
        sqlx::query_scalar("SELECT refreshed_through FROM taste_model_versions WHERE version = $1")
            .bind(version)
            .fetch_one(pool)
            .await?;
    Ok(mark.map(|mark| mark.and_utc().timestamp_micros()))
}

#[sqlx::test(migrations = "../api/migrations")]
async fn an_older_model_finishing_late_never_takes_over(pool: PgPool) -> anyhow::Result<()> {
    let older = "taste-202609201200-00000001";
    let newer = "taste-202609241200-00000002";
    record(&pool, older, "taste-input-a", 4).await?;
    record(&pool, newer, "taste-input-b", 0).await?;
    let computed_at = Utc::now();

    let newer_serves = activate(&pool, newer, trained(&pool, newer).await?, computed_at)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let older_serves = activate(&pool, older, trained(&pool, older).await?, computed_at)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let older_applied: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT applied_at FROM taste_model_versions WHERE version = $1")
            .bind(older)
            .fetch_one(&pool)
            .await?;

    assert!(newer_serves);
    assert!(!older_serves);
    assert_eq!(active(&pool).await?, vec![newer.to_owned()]);
    assert!(older_applied.is_some());
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_late_refresh_of_the_retired_model_leaves_the_new_mark_alone(
    pool: PgPool,
) -> anyhow::Result<()> {
    let retired = "taste-202609201200-00000001";
    let serving = "taste-202609241200-00000002";
    record(&pool, retired, "taste-input-a", 4).await?;
    record(&pool, serving, "taste-input-b", 0).await?;
    let before = Utc::now() - chrono::Duration::hours(2);
    let activated_at = Utc::now() - chrono::Duration::hours(1);
    activate(&pool, retired, trained(&pool, retired).await?, before)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    activate(&pool, serving, trained(&pool, serving).await?, activated_at)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    let late_until = Utc::now().naive_utc();
    sqlx::query_file!("queries/taste/finish_refresh.sql", retired, late_until)
        .execute(&pool)
        .await?;

    assert_eq!(
        mark(&pool, serving).await?,
        Some(activated_at.timestamp_micros())
    );
    assert_eq!(
        mark(&pool, retired).await?,
        Some(late_until.and_utc().timestamp_micros())
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_rolled_back_model_catches_up_from_where_it_stopped(pool: PgPool) -> anyhow::Result<()> {
    let restored = "taste-202609201200-00000001";
    let dropped = "taste-202609241200-00000002";
    record(&pool, restored, "taste-input-a", 4).await?;
    record(&pool, dropped, "taste-input-b", 0).await?;
    let stopped_at = Utc::now() - chrono::Duration::days(2);
    activate(&pool, restored, trained(&pool, restored).await?, stopped_at)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    activate(&pool, dropped, trained(&pool, dropped).await?, Utc::now())
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    sqlx::raw_sql(
        "UPDATE taste_model_versions SET active = false WHERE active;
         UPDATE taste_model_versions SET active = true
         WHERE version = 'taste-202609201200-00000001';",
    )
    .execute(&pool)
    .await?;
    let serving = sqlx::query_file!("queries/taste/active_version.sql")
        .fetch_one(&pool)
        .await?;
    let now = Utc::now().naive_utc();
    let (from, until) =
        super::super::refresh_window(serving.refreshed_through, now, chrono::Duration::minutes(5));

    assert_eq!(serving.version, restored);
    assert_eq!(
        from.and_utc().timestamp_micros(),
        stopped_at.timestamp_micros()
    );
    assert_eq!(until, from + chrono::Duration::days(1));
    Ok(())
}

#[test]
fn a_stale_version_behind_the_alias_is_skipped_and_the_rest_are_dropped() {
    let stale = |version: &str| StaleVersion {
        version: version.to_owned(),
        collection: collection_name(version),
    };
    let served = collection_name("taste-202609221200-00000003");

    let (droppable, behind_alias) = split_by_alias(
        vec![
            stale("taste-202609221200-00000003"),
            stale("taste-202609211200-00000002"),
            stale("taste-202609201200-00000001"),
        ],
        Some(&served),
    );

    assert_eq!(behind_alias, vec![stale("taste-202609221200-00000003")]);
    assert_eq!(
        droppable,
        vec![
            stale("taste-202609211200-00000002"),
            stale("taste-202609201200-00000001")
        ]
    );
}

#[test]
fn only_old_model_objects_without_a_version_row_are_orphans() {
    let now = 10_000_000;
    let day = 86_400;
    let object = |name: &str, age: i64| StoredObject {
        name: name.to_owned(),
        modified_unix: Some(now - age),
    };
    let known = HashSet::from(["taste-202609201200-00000001".to_owned()]);
    let objects = vec![
        object("taste-202609201200-00000001", 30 * day),
        object("taste-202609201200-00000001-tower", 30 * day),
        object("taste-202609211200-0000000a", 30 * day),
        object("taste-202609211200-0000000a-tower", 30 * day),
        object("taste-202609241200-0000000b-tower", day),
        object("notes.txt", 30 * day),
        StoredObject {
            name: "taste-202609191200-0000000c".to_owned(),
            modified_unix: None,
        },
    ];

    assert_eq!(
        orphan_objects(&objects, &known, now, 2 * day),
        vec![
            "taste-202609211200-0000000a".to_owned(),
            "taste-202609211200-0000000a-tower".to_owned()
        ]
    );
    assert!(orphan_age_s() > 39_900);
}

#[test]
fn a_result_that_cannot_be_applied_leaves_its_unrecorded_model_to_be_removed() {
    let known = HashSet::from(["taste-202609201200-00000001".to_owned()]);
    let mut recorded = result(WorkerStatus::Ok, None);
    recorded.version = Some("taste-202609201200-00000001".to_owned());
    let mut invalid_name = result(WorkerStatus::Ok, None);
    invalid_name.version = Some("latest".to_owned());

    assert_eq!(
        unrecorded_version(&result(WorkerStatus::Ok, None), &known),
        Some(VERSION)
    );
    assert_eq!(unrecorded_version(&recorded, &known), None);
    assert_eq!(unrecorded_version(&invalid_name, &known), None);
    assert!(ResultOutcome::ALL.contains(&ResultOutcome::Invalid));
    assert_eq!(ResultOutcome::Invalid.as_str(), "invalid");
}

#[sqlx::test(migrations = "../api/migrations")]
async fn stored_vectors_are_replaced_per_user_and_leave_with_their_version(
    pool: PgPool,
) -> anyhow::Result<()> {
    record(&pool, VERSION, "taste-input-a", 0).await?;
    let first = vec![("7".to_owned(), axis(0)), ("8".to_owned(), axis(1))];
    let second = vec![("7".to_owned(), axis(2))];

    vectors::store(&pool, VERSION, &first)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    vectors::store(&pool, VERSION, &second)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let seven: Vec<f32> = sqlx::query_scalar(
        "SELECT vec FROM user_taste_vectors WHERE sc_user_id = '7' AND version = $1",
    )
    .bind(VERSION)
    .fetch_one(&pool)
    .await?;
    let eight: Vec<f32> = sqlx::query_scalar(
        "SELECT vec FROM user_taste_vectors WHERE sc_user_id = '8' AND version = $1",
    )
    .bind(VERSION)
    .fetch_one(&pool)
    .await?;
    sqlx::query_file!("queries/taste/forget_version.sql", VERSION)
        .execute(&pool)
        .await?;
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM user_taste_vectors")
        .fetch_one(&pool)
        .await?;

    assert_eq!(seven, axis(2));
    assert_eq!(eight, axis(1));
    assert_eq!(left, 0);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_second_delivery_of_the_same_input_changes_nothing(pool: PgPool) -> anyhow::Result<()> {
    record(&pool, VERSION, "taste-input-a", 0).await?;
    sqlx::query_file!("queries/taste/mark_applied.sql", VERSION)
        .execute(&pool)
        .await?;

    let recorded = sqlx::query_file!("queries/taste/recorded_input.sql", "taste-input-a")
        .fetch_optional(&pool)
        .await?;
    let changed = sqlx::query_file!(
        "queries/taste/record_version.sql",
        VERSION,
        "taste-input-a",
        collection_name(VERSION),
        Utc::now(),
        128i16,
        serde_json::json!({"w": {"like": 2.0}, "tau_days": 1.0}),
        serde_json::json!({}),
        1i32,
        99i32
    )
    .execute(&pool)
    .await?
    .rows_affected();

    assert!(recorded.is_some_and(|row| row.applied_at.is_some()));
    assert_eq!(changed, 0);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn only_versions_beyond_the_kept_ones_are_stale(pool: PgPool) -> anyhow::Result<()> {
    for (day, version) in [
        (4, "taste-202609201200-00000001"),
        (3, "taste-202609211200-00000002"),
        (2, "taste-202609221200-00000003"),
        (1, "taste-202609231200-00000004"),
    ] {
        record(&pool, version, &format!("taste-input-{day}"), day).await?;
    }
    let newest = "taste-202609231200-00000004";
    activate(&pool, newest, trained(&pool, newest).await?, Utc::now())
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    let stale = sqlx::query_file!("queries/taste/stale_versions.sql", 2i64)
        .fetch_all(&pool)
        .await?;

    assert_eq!(
        stale.into_iter().map(|row| row.version).collect::<Vec<_>>(),
        vec!["taste-202609201200-00000001".to_owned()]
    );
    Ok(())
}

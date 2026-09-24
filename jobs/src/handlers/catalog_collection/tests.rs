use backend_contracts::{CatalogCollection, CatalogCollectionPayload, JobKind};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::{page, state, writer::CollectionWriter};
use crate::queue::LeasedJob;

async fn setup(
    pool: &PgPool,
) -> anyhow::Result<(LeasedJob, CatalogCollectionPayload, CollectionWriter)> {
    crate::db::migrations::run_core(pool, None).await?;
    let job = LeasedJob {
        id: Uuid::now_v7(),
        kind: JobKind::CatalogCollection,
        dedup_key: Some("liked-tracks:42:owner".into()),
        payload: json!({}),
        generation: 1,
        attempts: 1,
        max_attempts: 8,
        lease_id: Uuid::now_v7(),
    };
    sqlx::query("INSERT INTO background_jobs (id, kind, lane, lease_id, lease_generation, generation, lease_expires_at, leased_by, payload)
        VALUES ($1, 'catalog.collection', 'core_bulk', $2, 1, 1, now() + interval '1 hour', 'collection-test', '{}')")
        .bind(job.id).bind(job.lease_id).execute(pool).await?;
    let payload = CatalogCollectionPayload {
        collection: CatalogCollection::LikedTracks,
        subject_id: "42".into(),
        owner: true,
    };
    Ok((job, payload, CollectionWriter::new(pool.clone(), 420000)))
}

fn track(id: i64) -> Value {
    json!({"id": id, "urn": format!("soundcloud:tracks:{id}"), "title": "Track", "sharing": "public", "duration": 120000,
        "user": {"id": 42, "urn": "soundcloud:users:42", "username": "Owner"}})
}

fn page(ids: &[i64], next: Option<&str>) -> page::Page {
    page::Page {
        items: ids.iter().copied().map(track).collect(),
        next: next.map(str::to_owned),
        liked_at: Vec::new(),
    }
}

async fn like_times(pool: &PgPool) -> anyhow::Result<Vec<(String, Option<String>, String)>> {
    Ok(sqlx::query_as(
        "SELECT sc_track_id, to_char(liked_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS'), to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD')
         FROM user_likes_tracks WHERE user_id = '42' ORDER BY sc_track_id",
    )
    .fetch_all(pool)
    .await?)
}

async fn next_generation(pool: &PgPool, job: &mut LeasedJob) -> anyhow::Result<()> {
    job.generation += 1;
    sqlx::query("UPDATE background_jobs SET generation = $2, lease_generation = $2 WHERE id = $1")
        .bind(job.id)
        .bind(job.generation)
        .execute(pool)
        .await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_public_like_walk_backfills_the_real_like_time_and_later_snapshots_keep_it(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (mut job, mut payload, writer) = setup(&pool).await?;
    payload.owner = false;
    sqlx::query(
        "INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state, progress, synced_at, created_at)
         VALUES ('42', '7', true, false, now(), '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await?;
    let liked = json!({"collection": [
        {"created_at": "2019-10-06T06:37:19Z", "kind": "like", "track": track(7)},
        {"created_at": "2020-02-02T02:02:02Z", "kind": "like", "track": track(8)},
        {"kind": "like", "track": track(9)}
    ]});
    let snapshot = state::begin(&pool, &job, &payload)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing snapshot"))?;
    writer
        .persist(
            &job,
            &payload,
            &snapshot,
            page::parse(&payload, liked.clone(), true)?,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let expected = vec![
        (
            "7".to_owned(),
            Some("2019-10-06T06:37:19".to_owned()),
            "2026-01-01".to_owned(),
        ),
        (
            "8".to_owned(),
            Some("2020-02-02T02:02:02".to_owned()),
            like_times(&pool).await?[1].2.clone(),
        ),
        ("9".to_owned(), None, like_times(&pool).await?[2].2.clone()),
    ];
    assert_eq!(like_times(&pool).await?, expected);

    next_generation(&pool, &mut job).await?;
    let repeated = state::begin(&pool, &job, &payload)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing repeated snapshot"))?;
    writer
        .persist(
            &job,
            &payload,
            &repeated,
            page::parse(&payload, liked, true)?,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    assert_eq!(like_times(&pool).await?, expected);

    next_generation(&pool, &mut job).await?;
    payload.owner = true;
    let owner = state::begin(&pool, &job, &payload)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing owner snapshot"))?;
    writer
        .persist(
            &job,
            &payload,
            &owner,
            page::parse(
                &payload,
                json!({"collection": [track(7), track(8), track(9)]}),
                false,
            )?,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    assert_eq!(like_times(&pool).await?, expected);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn followers_prune_only_after_complete_public_snapshots_including_empty_ones(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (mut job, mut payload, writer) = setup(&pool).await?;
    payload.collection = CatalogCollection::Followers;
    payload.owner = false;
    sqlx::query(
        "INSERT INTO user_followers (user_id, target_user_urn, synced_at, created_at)
        VALUES ('42', 'soundcloud:users:99', now() - interval '1 day', now() - interval '1 day'),
               ('43', 'soundcloud:users:99', now() - interval '1 day', now() - interval '1 day')",
    )
    .execute(&pool)
    .await?;
    let first = state::begin(&pool, &job, &payload)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing snapshot"))?;
    let response = json!({"collection":[{"urn":"soundcloud:users:17","username":"Follower"}],
        "next_href":"https://api.soundcloud.com/users/42/followers?offset=100"});
    writer
        .persist(
            &job,
            &payload,
            &first,
            page::parse(&payload, response, false)?,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let partial: i64 =
        sqlx::query_scalar("SELECT count(*) FROM user_followers WHERE user_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(partial, 2);
    let second = state::begin(&pool, &job, &payload)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing continuation"))?;
    writer
        .persist(
            &job,
            &payload,
            &second,
            page::parse(
                &payload,
                json!({"collection":[{"urn":"soundcloud:users:18","username":"Last"}]}),
                false,
            )?,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let members: Vec<String> = sqlx::query_scalar(
        "SELECT target_user_urn FROM user_followers WHERE user_id = '42' ORDER BY target_user_urn",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(members, ["soundcloud:users:17", "soundcloud:users:18"]);
    let done: bool = sqlx::query_scalar("SELECT complete AND synced_at IS NOT NULL FROM catalog_collection_sync WHERE subject_id = '42'")
        .fetch_one(&pool).await?;
    assert!(done);
    job.generation += 1;
    sqlx::query("UPDATE background_jobs SET generation = $2, lease_generation = $2 WHERE id = $1")
        .bind(job.id)
        .bind(job.generation)
        .execute(&pool)
        .await?;
    let empty = state::begin(&pool, &job, &payload)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing new snapshot"))?;
    writer
        .persist(
            &job,
            &payload,
            &empty,
            page::parse(&payload, json!({"collection":[]}), false)?,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let survivors: Vec<String> = sqlx::query_scalar("SELECT user_id FROM user_followers")
        .fetch_all(&pool)
        .await?;
    assert_eq!(survivors, ["43"]);
    let users: i64 =
        sqlx::query_scalar("SELECT count(*) FROM users WHERE sc_user_id IN ('17','18')")
            .fetch_one(&pool)
            .await?;
    assert_eq!(users, 2);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn owner_followings_schedule_public_uploads_without_reviving_unfollows_or_resetting_retry(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (job, mut payload, writer) = setup(&pool).await?;
    payload.collection = CatalogCollection::Followings;
    sqlx::query("INSERT INTO user_followings (user_id, target_user_urn, wanted_state, progress) VALUES
        ('42', 'soundcloud:users:18', false, true), ('soundcloud:users:42', 'soundcloud:users:18', true, false)")
        .execute(&pool).await?;
    sqlx::query(
        "INSERT INTO catalog_collection_sync (subject_id, collection, scope, job_id, generation, snapshot_id, synced_at, complete)
        VALUES ('19', 'owned-tracks', 'public', $1, 1, $2, now(), true)",
    )
    .bind(job.id)
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await?;
    let snapshot = state::begin(&pool, &job, &payload)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing snapshot"))?;
    let response = json!({"collection":[
        {"id":17,"urn":"soundcloud:users:17","username":"New"},
        {"id":18,"urn":"soundcloud:users:18","username":"Unfollowed"},
        {"id":19,"urn":"soundcloud:users:19","username":"Fresh"}
    ]});
    writer
        .persist(
            &job,
            &payload,
            &snapshot,
            page::parse(&payload, response.clone(), false)?,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let children: Vec<(String, Value)> = sqlx::query_as(
        "SELECT dedup_key, payload FROM background_jobs WHERE dedup_key LIKE 'owned-tracks:%'",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        children,
        vec![(
            "owned-tracks:17:public".into(),
            json!({"version":"1","payload":{"collection":"owned-tracks","subject_id":"17","owner":false}})
        )]
    );
    sqlx::query("UPDATE background_jobs SET attempts = 3, available_at = now() + interval '1 hour' WHERE dedup_key = 'owned-tracks:17:public'")
        .execute(&pool).await?;
    payload.subject_id = "43".into();
    let snapshot = state::begin(&pool, &job, &payload)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing second subscriber snapshot"))?;
    writer
        .persist(
            &job,
            &payload,
            &snapshot,
            page::parse(&payload, response, false)?,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let preserved: (i32, bool) = sqlx::query_as("SELECT attempts, available_at > now() + interval '30 minutes' FROM background_jobs WHERE dedup_key = 'owned-tracks:17:public'")
        .fetch_one(&pool).await?;
    assert_eq!(preserved, (3, true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn public_followings_and_expired_leases_do_not_fan_out_upload_jobs(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (job, mut payload, writer) = setup(&pool).await?;
    payload.collection = CatalogCollection::Followings;
    payload.owner = false;
    let snapshot = state::begin(&pool, &job, &payload)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing snapshot"))?;
    let response =
        json!({"collection":[{"id":17,"urn":"soundcloud:users:17","username":"Public"}]});
    writer
        .persist(
            &job,
            &payload,
            &snapshot,
            page::parse(&payload, response.clone(), false)?,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    payload.owner = true;
    let owner_snapshot = state::begin(&pool, &job, &payload)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing snapshot"))?;
    sqlx::query(
        "UPDATE background_jobs SET lease_expires_at = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(job.id)
    .execute(&pool)
    .await?;
    writer
        .persist(
            &job,
            &payload,
            &owner_snapshot,
            page::parse(&payload, response, false)?,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let children: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM background_jobs WHERE dedup_key LIKE 'owned-tracks:%'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(children, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn pages_resume_atomically_and_duplicate_delivery_cannot_rewind_progress(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (job, payload, writer) = setup(&pool).await?;
    sqlx::query("UPDATE background_jobs SET attempts = 7 WHERE id = $1")
        .bind(job.id)
        .execute(&pool)
        .await?;
    let first = state::begin(&pool, &job, &payload).await?.unwrap();
    writer
        .persist(
            &job,
            &payload,
            &first,
            page(&[1], Some("v1:/me/likes/tracks?offset=100")),
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    writer
        .persist(
            &job,
            &payload,
            &first,
            page(&[99], None),
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let resumed = state::begin(&pool, &job, &payload).await?.unwrap();
    assert_eq!(
        resumed.next_cursor.as_deref(),
        Some("v1:/me/likes/tracks?offset=100")
    );
    assert_eq!(
        (resumed.page_count, resumed.item_count, resumed.complete),
        (1, 1, false)
    );
    writer
        .persist(
            &job,
            &payload,
            &resumed,
            page(&[2], None),
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let done = state::begin(&pool, &job, &payload).await?.unwrap();
    assert_eq!(
        (done.page_count, done.item_count, done.complete),
        (2, 2, true)
    );
    let tracks: Vec<String> =
        sqlx::query_scalar("SELECT sc_track_id FROM tracks ORDER BY sc_track_id")
            .fetch_all(&pool)
            .await?;
    assert_eq!(tracks, ["1", "2"]);
    let indexed: i64 = sqlx::query_scalar("SELECT count(*) FROM background_jobs WHERE kind = $1")
        .bind(JobKind::IndexTrack.as_str())
        .fetch_one(&pool)
        .await?;
    assert_eq!(indexed, 2);
    let order: Vec<String> =
        sqlx::query_scalar("SELECT sc_track_id FROM user_likes_tracks ORDER BY created_at DESC")
            .fetch_all(&pool)
            .await?;
    assert_eq!(order, ["1", "2"]);
    let attempts: i32 = sqlx::query_scalar("SELECT attempts FROM background_jobs WHERE id = $1")
        .bind(job.id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(attempts, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn failed_page_rolls_back_catalog_mirror_cursor_and_downstream_work(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (job, payload, writer) = setup(&pool).await?;
    let snapshot = state::begin(&pool, &job, &payload).await?.unwrap();
    sqlx::query(
        "ALTER TABLE user_likes_tracks ADD CONSTRAINT reject_track CHECK (sc_track_id <> '2')",
    )
    .execute(&pool)
    .await?;
    assert!(
        writer
            .persist(
                &job,
                &payload,
                &snapshot,
                page(&[1, 2], Some("v1:/me/likes/tracks?offset=100")),
                catalog_ingest::Observation::begin(&pool).await?
            )
            .await
            .is_err()
    );
    for table in [
        "tracks",
        "user_likes_tracks",
        "catalog_collection_seen",
        "catalog_collection_cursors",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&pool)
            .await?;
        assert_eq!(count, 0, "{table}");
    }
    let resumed = state::begin(&pool, &job, &payload).await?.unwrap();
    assert_eq!(resumed.page_count, 0);
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM background_jobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(jobs, 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn repeated_cursor_and_lost_generation_cannot_commit_a_page(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (job, payload, writer) = setup(&pool).await?;
    let first = state::begin(&pool, &job, &payload).await?.unwrap();
    let cursor = "v1:/me/likes/tracks?offset=100";
    writer
        .persist(
            &job,
            &payload,
            &first,
            page(&[1], Some(cursor)),
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let resumed = state::begin(&pool, &job, &payload).await?.unwrap();
    assert!(
        writer
            .persist(
                &job,
                &payload,
                &resumed,
                page(&[2], Some(cursor)),
                catalog_ingest::Observation::begin(&pool).await?
            )
            .await
            .is_err()
    );
    sqlx::query("UPDATE background_jobs SET generation = 2 WHERE id = $1")
        .bind(job.id)
        .execute(&pool)
        .await?;
    writer
        .persist(
            &job,
            &payload,
            &resumed,
            page(&[3], None),
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT sc_track_id FROM tracks ORDER BY sc_track_id")
            .fetch_all(&pool)
            .await?;
    assert_eq!(ids, ["1"]);
    assert!(state::begin(&pool, &job, &payload).await?.is_none());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn observation_preserves_pending_local_intent_and_public_scope_cannot_delete(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (job, mut payload, writer) = setup(&pool).await?;
    sqlx::query("INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state, progress, synced_at, created_at)
        VALUES ('42', '1', false, true, now() - interval '1 day', now() - interval '1 day'),
        ('42', '2', true, true, now() - interval '1 day', now() - interval '1 day'),
        ('42', '3', true, false, now() - interval '1 day', now() - interval '1 day')").execute(&pool).await?;
    let snapshot = state::begin(&pool, &job, &payload).await?.unwrap();
    writer
        .persist(
            &job,
            &payload,
            &snapshot,
            page(&[1, 2], None),
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let rows: Vec<(String, bool, bool)> = sqlx::query_as(
        "SELECT sc_track_id, wanted_state, progress FROM user_likes_tracks ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(rows, [("1".into(), false, true), ("2".into(), true, true)]);
    payload.owner = false;
    sqlx::query("INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state, progress, synced_at, created_at)
        VALUES ('42', '3', true, false, now() - interval '1 day', now() - interval '1 day')").execute(&pool).await?;
    let public = state::begin(&pool, &job, &payload).await?.unwrap();
    writer
        .persist(
            &job,
            &payload,
            &public,
            page(&[1, 2], None),
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM user_likes_tracks")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 3);
    let scopes: i64 =
        sqlx::query_scalar("SELECT count(*) FROM catalog_collection_sync WHERE complete")
            .fetch_one(&pool)
            .await?;
    assert_eq!(scopes, 2);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn every_collection_kind_persists_to_its_own_mirror(pool: PgPool) -> anyhow::Result<()> {
    let (job, mut payload, writer) = setup(&pool).await?;
    for (kind, table, item) in [
        (
            CatalogCollection::OwnedTracks,
            "user_owned_tracks",
            track(1),
        ),
        (
            CatalogCollection::OwnedPlaylists,
            "user_owned_playlists",
            json!({"urn": "soundcloud:playlists:2", "title": "Set", "sharing": "private", "track_count": 0, "user": {"urn": "soundcloud:users:42", "username": "Owner"}}),
        ),
        (
            CatalogCollection::LikedPlaylists,
            "user_likes_playlists",
            json!({"urn": "soundcloud:playlists:2", "title": "Set", "sharing": "private", "track_count": 0, "user": {"urn": "soundcloud:users:42", "username": "Owner"}}),
        ),
        (
            CatalogCollection::Followings,
            "user_followings",
            json!({"urn": "soundcloud:users:3", "username": "Artist"}),
        ),
    ] {
        payload.collection = kind;
        let snapshot = state::begin(&pool, &job, &payload).await?.unwrap();
        let parsed = page::parse(&payload, json!({"collection": [item]}), false)?;
        writer
            .persist(
                &job,
                &payload,
                &snapshot,
                parsed,
                catalog_ingest::Observation::begin(&pool).await?,
            )
            .await?;
        let count: i64 = sqlx::query_scalar(&format!(
            "SELECT count(*) FROM {table} WHERE user_id = '42'"
        ))
        .fetch_one(&pool)
        .await?;
        assert_eq!(count, 1, "{table}");
    }
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_audience_snapshot_prunes_only_its_own_relation_and_only_when_complete(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (mut job, mut payload, writer) = setup(&pool).await?;
    payload.collection = CatalogCollection::TrackFavoriters;
    payload.owner = false;
    sqlx::query(
        "INSERT INTO catalog_audience (subject_urn, relation, user_urn, created_at) VALUES
        ('soundcloud:tracks:42', 'track-favoriters', 'soundcloud:users:99', now() - interval '1 day'),
        ('soundcloud:tracks:42', 'track-reposters', 'soundcloud:users:99', now() - interval '1 day'),
        ('soundcloud:tracks:43', 'track-favoriters', 'soundcloud:users:99', now() - interval '1 day')",
    )
    .execute(&pool)
    .await?;
    let first = state::begin(&pool, &job, &payload)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing snapshot"))?;
    let response = json!({"collection":[{"urn":"soundcloud:users:17","username":"Fan"}],
        "next_href":"https://api.soundcloud.com/tracks/42/favoriters?offset=100"});
    writer
        .persist(
            &job,
            &payload,
            &first,
            page::parse(&payload, response, false)?,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let stale: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM catalog_audience WHERE user_urn = 'soundcloud:users:99'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(stale, 3);
    let second = state::begin(&pool, &job, &payload)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing continuation"))?;
    writer
        .persist(
            &job,
            &payload,
            &second,
            page::parse(
                &payload,
                json!({"collection":[{"urn":"soundcloud:users:18","username":"Later"}]}),
                false,
            )?,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let ordered: Vec<String> = sqlx::query_scalar(
        "SELECT user_urn FROM catalog_audience
         WHERE subject_urn = 'soundcloud:tracks:42' AND relation = 'track-favoriters'
         ORDER BY created_at DESC, user_urn DESC",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(ordered, ["soundcloud:users:17", "soundcloud:users:18"]);
    let untouched: Vec<String> = sqlx::query_scalar(
        "SELECT relation || '@' || subject_urn FROM catalog_audience
         WHERE user_urn = 'soundcloud:users:99' ORDER BY 1",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        untouched,
        [
            "track-favoriters@soundcloud:tracks:43",
            "track-reposters@soundcloud:tracks:42"
        ]
    );
    let known: i64 =
        sqlx::query_scalar("SELECT count(*) FROM users WHERE sc_user_id IN ('17','18')")
            .fetch_one(&pool)
            .await?;
    assert_eq!(known, 2);
    job.generation += 1;
    sqlx::query("UPDATE background_jobs SET generation = $2, lease_generation = $2 WHERE id = $1")
        .bind(job.id)
        .bind(job.generation)
        .execute(&pool)
        .await?;
    let empty = state::begin(&pool, &job, &payload)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing new snapshot"))?;
    writer
        .persist(
            &job,
            &payload,
            &empty,
            page::parse(&payload, json!({"collection":[]}), false)?,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let left: Vec<String> = sqlx::query_scalar(
        "SELECT subject_urn FROM catalog_audience WHERE relation = 'track-favoriters' ORDER BY 1",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(left, ["soundcloud:tracks:43"]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn playlist_reposters_address_the_playlist_and_keep_their_own_mirror(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (job, mut payload, writer) = setup(&pool).await?;
    payload.collection = CatalogCollection::PlaylistReposters;
    payload.owner = false;
    let snapshot = state::begin(&pool, &job, &payload)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing snapshot"))?;
    writer
        .persist(
            &job,
            &payload,
            &snapshot,
            page::parse(
                &payload,
                json!({"collection":[{"urn":"soundcloud:users:7","username":"Sharer"}]}),
                false,
            )?,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let rows: Vec<String> =
        sqlx::query_scalar("SELECT subject_urn || '/' || user_urn FROM catalog_audience")
            .fetch_all(&pool)
            .await?;
    assert_eq!(rows, ["soundcloud:playlists:42/soundcloud:users:7"]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn comments_keep_optimistic_local_writes_until_the_queue_drains(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (mut job, mut payload, writer) = setup(&pool).await?;
    payload.collection = CatalogCollection::TrackComments;
    payload.owner = false;
    sqlx::raw_sql(
        "INSERT INTO users (sc_user_id, urn, username, username_normalized)
         VALUES ('5', 'soundcloud:users:5', 'Author', 'author');
         INSERT INTO track_comments (id, sc_track_id, user_urn, body, created_at)
         VALUES (gen_random_uuid(), '42', 'soundcloud:users:5', 'waiting', now() - interval '1 day'),
                (gen_random_uuid(), '42', 'soundcloud:users:9', 'abandoned', now() - interval '1 day');
         INSERT INTO sync_queue (id, user_id, action_type, target_urn)
         VALUES (gen_random_uuid(), '5', 'comment', 'soundcloud:tracks:42');",
    )
    .execute(&pool)
    .await?;
    let snapshot = state::begin(&pool, &job, &payload)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing snapshot"))?;
    let response = json!({"collection": [
        {"id": 7, "kind": "comment", "body": "nice", "timestamp": 1500,
         "created_at": "2024/05/01 12:00:00 +0000", "track_id": 42,
         "user": {"id": 5, "kind": "user", "username": "Author"}}
    ]});
    writer
        .persist(
            &job,
            &payload,
            &snapshot,
            page::parse(&payload, response, true)?,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let kept: Vec<String> = sqlx::query_scalar(
        "SELECT body FROM track_comments WHERE sc_track_id = '42' ORDER BY created_at DESC",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(kept, ["nice", "waiting"]);
    let stored: (Option<i64>, Option<String>) = sqlx::query_as(
        "SELECT track_position_ms, to_char(sc_created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS')
         FROM track_comments WHERE sc_comment_id = '7'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(stored.0, Some(1500));
    assert_eq!(stored.1.as_deref(), Some("2024-05-01 12:00:00"));
    job.generation += 1;
    sqlx::query("UPDATE background_jobs SET generation = $2, lease_generation = $2 WHERE id = $1")
        .bind(job.id)
        .bind(job.generation)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM sync_queue").execute(&pool).await?;
    let second = state::begin(&pool, &job, &payload)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing snapshot"))?;
    writer
        .persist(
            &job,
            &payload,
            &second,
            page::parse(&payload, json!({"collection": []}), true)?,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM track_comments")
        .fetch_one(&pool)
        .await?;
    assert_eq!(left, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_malformed_comment_page_cannot_certify_a_snapshot(pool: PgPool) -> anyhow::Result<()> {
    let (_job, mut payload, _writer) = setup(&pool).await?;
    payload.collection = CatalogCollection::TrackComments;
    payload.owner = false;
    let author = json!({"id": 5, "kind": "user", "username": "Author"});
    for value in [
        json!({"collection": [{"id": 7, "body": "hi"}]}),
        json!({"collection": [{"id": 0, "body": "hi", "user": author}]}),
        json!({"collection": [{"id": 7, "body": "  ", "user": author}]}),
        json!({"collection": [{"id": 7, "body": "hi", "user": {"id": 5}}]}),
        json!({"collection": [{"id": 7, "body": "hi", "user": author, "track_id": 43}]}),
        json!({"collection": [{"id": 7, "body": "hi", "user": author, "timestamp": -5}]}),
        json!({"collection": [{"id": 7, "body": "hi", "user": author, "created_at": "yesterday"}]}),
    ] {
        assert!(
            page::parse(&payload, value.clone(), true).is_err(),
            "{value}"
        );
    }
    let ok = page::parse(
        &payload,
        json!({"collection": [{"id": 7, "body": "hi", "user": author, "track_id": 42}]}),
        true,
    )?;
    assert_eq!(ok.items[0]["id"], "7");
    assert_eq!(ok.items[0]["track_id"], "42");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_page_repeating_one_comment_still_commits(pool: PgPool) -> anyhow::Result<()> {
    let (job, mut payload, writer) = setup(&pool).await?;
    payload.collection = CatalogCollection::TrackComments;
    payload.owner = false;
    let snapshot = state::begin(&pool, &job, &payload)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing snapshot"))?;
    let author = json!({"id": 5, "kind": "user", "username": "Author"});
    let response = json!({"collection": [
        {"id": 7, "body": "first", "user": author, "track_id": 42},
        {"id": 7, "body": "repeat", "user": author, "track_id": 42}
    ]});
    writer
        .persist(
            &job,
            &payload,
            &snapshot,
            page::parse(&payload, response, true)?,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let stored: Vec<String> = sqlx::query_scalar("SELECT body FROM track_comments")
        .fetch_all(&pool)
        .await?;
    assert_eq!(stored, ["first"]);
    Ok(())
}

use std::time::Duration;

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::model::ClaimedMutation;
use super::repository::SyncQueueRepository;

const USER: &str = "17";
const TRACK: &str = "42";
const TARGET: &str = "soundcloud:tracks:42";

async fn setup(pool: &PgPool) -> anyhow::Result<SyncQueueRepository> {
    crate::db::migrations::run_core(pool, None).await?;
    super::track_tests::connect_owner(pool).await?;
    Ok(SyncQueueRepository::new(
        pool.clone(),
        Duration::from_secs(60),
    ))
}

async fn tap(pool: &PgPool, like: bool) -> anyhow::Result<()> {
    let (action, inverse) = if like {
        ("like_track", "unlike_track")
    } else {
        ("unlike_track", "like_track")
    };
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL scd.writer = 'api'")
        .execute(&mut *tx)
        .await?;
    let mirror = if like {
        "INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state) VALUES ($1, $2, true)
         ON CONFLICT (user_id, sc_track_id) DO UPDATE SET
             wanted_state = true,
             created_at = CASE WHEN user_likes_tracks.wanted_state THEN user_likes_tracks.created_at ELSE now() END,
             wanted_v = public.scd_hlc(user_likes_tracks.wanted_v)"
    } else {
        "INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state) VALUES ($1, $2, false)
         ON CONFLICT (user_id, sc_track_id) DO UPDATE SET
             wanted_state = false,
             wanted_v = public.scd_hlc(user_likes_tracks.wanted_v)"
    };
    sqlx::query(mirror)
        .bind(USER)
        .bind(TRACK)
        .execute(&mut *tx)
        .await?;
    let annuls = sqlx::query_file!(
        "../api/queries/sync_queue/service/cancel_inverse.sql",
        USER,
        inverse,
        TARGET
    )
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    sqlx::query(include_str!(
        "../../../../api/queries/sync_queue/service/enqueue.sql"
    ))
    .bind(USER)
    .bind(action)
    .bind(TARGET)
    .bind(None::<Value>)
    .bind(annuls)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn late(pool: &PgPool, action: &str, generation: i64) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL scd.writer = 'api'")
        .execute(&mut *tx)
        .await?;
    sqlx::query(include_str!(
        "../../../../api/queries/sync_queue/service/enqueue.sql"
    ))
    .bind(USER)
    .bind(action)
    .bind(TARGET)
    .bind(None::<Value>)
    .bind(false)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE sync_queue SET generation = $1 WHERE action_type = $2")
        .bind(generation)
        .bind(action)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn queue(pool: &PgPool) -> anyhow::Result<Vec<(String, bool)>> {
    Ok(
        sqlx::query_as("SELECT action_type, cancelled FROM sync_queue ORDER BY action_type")
            .fetch_all(pool)
            .await?,
    )
}

async fn mirror(pool: &PgPool) -> anyhow::Result<(bool, bool, bool)> {
    Ok(sqlx::query_as(
        "SELECT wanted_state, synced_for_v < wanted_v, wanted_v > 0 FROM user_likes_tracks
         WHERE user_id = $1 AND sc_track_id = $2",
    )
    .bind(USER)
    .bind(TRACK)
    .fetch_one(pool)
    .await?)
}

async fn receipts(pool: &PgPool, tbl: &str) -> anyhow::Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM repl_purges WHERE tbl = $1")
            .bind(tbl)
            .fetch_one(pool)
            .await?,
    )
}

async fn run_on_core(repository: &SyncQueueRepository) -> anyhow::Result<Vec<ClaimedMutation>> {
    repository.adopt().await?;
    repository.settle_cancelled().await?;
    Ok(repository.claim(10).await?)
}

async fn sent(repository: &SyncQueueRepository) -> anyhow::Result<Vec<String>> {
    let mut sent = Vec::new();
    for _ in 0..4 {
        for mutation in run_on_core(repository).await? {
            if repository.drop_superseded(&mutation).await? {
                continue;
            }
            sent.push(mutation.action_type.clone());
            complete(repository, &mutation).await?;
        }
    }
    Ok(sent)
}

async fn regress(pool: &PgPool, generation: i64, cancel: bool) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL scd.writer = 'api'")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE sync_queue SET generation = $1, cancelled = $2 WHERE action_type = 'like_track'",
    )
    .bind(generation)
    .bind(cancel)
    .execute(&mut *tx)
    .await?;
    if cancel {
        sqlx::query(include_str!(
            "../../../../api/queries/sync_queue/service/enqueue.sql"
        ))
        .bind(USER)
        .bind("unlike_track")
        .bind(TARGET)
        .bind(None::<Value>)
        .bind(true)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE user_likes_tracks SET wanted_state = false, wanted_v = public.scd_hlc(wanted_v)
             WHERE user_id = $1 AND sc_track_id = $2",
        )
        .bind(USER)
        .bind(TRACK)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn complete(
    repository: &SyncQueueRepository,
    mutation: &ClaimedMutation,
) -> anyhow::Result<()> {
    assert!(repository.record_remote_attempt(mutation).await?);
    assert!(
        repository
            .record_remote_success(mutation, &json!({"ok": true}))
            .await?
    );
    repository.finalize(mutation).await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_like_undone_before_any_claim_sends_only_the_unlike_the_mirror_wants(
    pool: PgPool,
) -> anyhow::Result<()> {
    let repository = setup(&pool).await?;
    tap(&pool, true).await?;
    tap(&pool, false).await?;
    assert_eq!(
        queue(&pool).await?,
        vec![
            ("like_track".to_owned(), true),
            ("unlike_track".to_owned(), true)
        ]
    );

    let claimed = run_on_core(&repository).await?;

    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].action_type, "unlike_track");
    assert_eq!(receipts(&pool, "sync_queue").await?, 1);
    complete(&repository, &claimed[0]).await?;
    assert!(queue(&pool).await?.is_empty());
    assert_eq!(mirror(&pool).await?, (false, false, true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn like_unlike_like_before_a_claim_still_sends_the_last_like(
    pool: PgPool,
) -> anyhow::Result<()> {
    let repository = setup(&pool).await?;
    tap(&pool, false).await?;
    tap(&pool, true).await?;
    tap(&pool, false).await?;
    assert_eq!(
        queue(&pool).await?,
        vec![
            ("like_track".to_owned(), true),
            ("unlike_track".to_owned(), false)
        ]
    );

    let claimed = run_on_core(&repository).await?;

    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].action_type, "unlike_track");
    assert_eq!(
        queue(&pool).await?,
        vec![("unlike_track".to_owned(), false)]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unlike_that_annulled_a_like_core_already_claimed_runs_after_it(
    pool: PgPool,
) -> anyhow::Result<()> {
    let repository = setup(&pool).await?;
    tap(&pool, true).await?;
    let like = run_on_core(&repository).await?.remove(0);
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL scd.writer = 'api'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE sync_queue SET cancelled = true WHERE action_type = 'like_track'")
        .execute(&mut *tx)
        .await?;
    sqlx::query(include_str!(
        "../../../../api/queries/sync_queue/service/enqueue.sql"
    ))
    .bind(USER)
    .bind("unlike_track")
    .bind(TARGET)
    .bind(None::<Value>)
    .bind(true)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE user_likes_tracks SET wanted_state = false WHERE user_id = $1 AND sc_track_id = $2",
    )
    .bind(USER)
    .bind(TRACK)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    assert!(run_on_core(&repository).await?.is_empty());
    assert_eq!(
        queue(&pool).await?,
        vec![
            ("like_track".to_owned(), true),
            ("unlike_track".to_owned(), false)
        ]
    );
    complete(&repository, &like).await?;
    let unlike = run_on_core(&repository).await?;
    assert_eq!(unlike.len(), 1);
    assert_eq!(unlike[0].action_type, "unlike_track");
    complete(&repository, &unlike[0]).await?;

    assert!(queue(&pool).await?.is_empty());
    assert_eq!(mirror(&pool).await?, (false, false, true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unlike_that_annulled_a_like_core_already_sent_and_purged_is_still_sent(
    pool: PgPool,
) -> anyhow::Result<()> {
    let repository = setup(&pool).await?;
    tap(&pool, true).await?;
    let like = run_on_core(&repository).await?.remove(0);
    complete(&repository, &like).await?;
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL scd.writer = 'api'")
        .execute(&mut *tx)
        .await?;
    sqlx::query(include_str!(
        "../../../../api/queries/sync_queue/service/enqueue.sql"
    ))
    .bind(USER)
    .bind("unlike_track")
    .bind(TARGET)
    .bind(None::<Value>)
    .bind(true)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE user_likes_tracks SET wanted_state = false WHERE user_id = $1 AND sc_track_id = $2",
    )
    .bind(USER)
    .bind(TRACK)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let unlike = run_on_core(&repository).await?;
    assert_eq!(unlike.len(), 1);
    assert_eq!(unlike[0].action_type, "unlike_track");

    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL scd.writer = 'api'")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO sync_queue (id, user_id, action_type, target_urn, generation, cancelled)
         VALUES ($1, $2, 'like_track', $3, $4, true)",
    )
    .bind(like.id)
    .bind(USER)
    .bind(TARGET)
    .bind(like.generation)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    assert!(run_on_core(&repository).await?.is_empty());
    assert_eq!(
        queue(&pool).await?,
        vec![("unlike_track".to_owned(), false)]
    );
    complete(&repository, &unlike[0]).await?;

    assert!(queue(&pool).await?.is_empty());
    assert_eq!(mirror(&pool).await?, (false, false, true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_like_after_an_unlike_that_was_sent_and_requeued_is_not_annulled_away(
    pool: PgPool,
) -> anyhow::Result<()> {
    let repository = setup(&pool).await?;
    tap(&pool, true).await?;
    let like = run_on_core(&repository).await?.remove(0);
    complete(&repository, &like).await?;
    tap(&pool, false).await?;
    let unlike = run_on_core(&repository).await?.remove(0);
    assert!(repository.record_remote_attempt(&unlike).await?);
    tap(&pool, false).await?;
    assert!(
        !repository
            .record_remote_success(&unlike, &json!({"ok": true}))
            .await?
    );
    repository.release(&unlike).await?;
    repository.adopt().await?;
    tap(&pool, true).await?;
    assert_eq!(
        queue(&pool).await?,
        vec![
            ("like_track".to_owned(), false),
            ("unlike_track".to_owned(), false)
        ]
    );

    assert_eq!(sent(&repository).await?, ["like_track"]);
    assert!(queue(&pool).await?.is_empty());
    assert_eq!(mirror(&pool).await?, (true, false, true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_new_intent_on_a_dead_row_is_adopted_before_the_claim(
    pool: PgPool,
) -> anyhow::Result<()> {
    let repository = setup(&pool).await?;
    tap(&pool, true).await?;
    let like = run_on_core(&repository).await?.remove(0);
    assert!(repository.park(&like, 9, "refused").await?);
    tap(&pool, true).await?;
    assert!(repository.claim(10).await?.is_empty());

    let claimed = run_on_core(&repository).await?;

    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].retry_count, 0);
    assert!(claimed[0].generation > like.generation);
    let exec: (Option<i64>, bool) =
        sqlx::query_as("SELECT exec_generation, dead FROM sync_queue WHERE id = $1")
            .bind(claimed[0].id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(exec, (Some(claimed[0].generation), false));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_served_unlike_is_purged_after_a_week_and_a_new_like_lands_above_the_receipt(
    pool: PgPool,
) -> anyhow::Result<()> {
    let repository = setup(&pool).await?;
    tap(&pool, true).await?;
    let like = run_on_core(&repository).await?.remove(0);
    complete(&repository, &like).await?;
    tap(&pool, false).await?;
    let unlike = run_on_core(&repository).await?.remove(0);
    complete(&repository, &unlike).await?;
    assert_eq!(mirror(&pool).await?, (false, false, true));
    assert_eq!(receipts(&pool, "sync_queue").await?, 2);

    let maintenance = crate::handlers::maintenance::MaintenanceHandler::new(pool.clone());
    maintenance
        .heal_sync_queue()
        .await
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    assert_eq!(receipts(&pool, "user_likes_tracks").await?, 0);
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE user_likes_tracks
         SET wanted_v = ((extract(epoch FROM now() - interval '8 days') * 1000000)::bigint << 4) + 2,
             synced_for_v = ((extract(epoch FROM now() - interval '8 days') * 1000000)::bigint << 4) + 2",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    maintenance
        .heal_sync_queue()
        .await
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    assert_eq!(receipts(&pool, "user_likes_tracks").await?, 1);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM user_likes_tracks")
        .fetch_one(&pool)
        .await?;
    assert_eq!(left, 0);

    tap(&pool, true).await?;
    let above: bool = sqlx::query_scalar(
        "SELECT m.wanted_v > p.purged_v FROM user_likes_tracks m, repl_purges p
         WHERE p.tbl = 'user_likes_tracks'",
    )
    .fetch_one(&pool)
    .await?;
    assert!(above);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn heal_brings_back_a_lost_like_above_its_queue_receipt(pool: PgPool) -> anyhow::Result<()> {
    let repository = setup(&pool).await?;
    tap(&pool, true).await?;
    let like = run_on_core(&repository).await?.remove(0);
    let id: Uuid = like.id;
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE user_likes_tracks
         SET wanted_v = ((extract(epoch FROM now() - interval '1 hour') * 1000000)::bigint << 4) + 2",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let seen: i64 = sqlx::query_scalar("SELECT intent_v FROM sync_queue WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await?;
    let purged: bool = sqlx::query_file_scalar!("queries/sync_queue/purge.sql", id, seen)
        .fetch_one(&pool)
        .await?;
    assert!(purged);

    crate::handlers::maintenance::MaintenanceHandler::new(pool.clone())
        .heal_sync_queue()
        .await
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;

    let healed: (Uuid, bool, Option<i64>, i64) = sqlx::query_as(
        "SELECT q.id, q.intent_v > p.purged_v, q.exec_generation, q.generation
         FROM sync_queue q, repl_purges p WHERE p.tbl = 'sync_queue'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(healed.0, id);
    assert!(healed.1);
    assert_eq!(healed.2, Some(healed.3));
    assert_eq!(run_on_core(&repository).await?.len(), 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_older_unlike_that_reaches_core_after_a_newer_like_ran_is_dropped_unsent(
    pool: PgPool,
) -> anyhow::Result<()> {
    let repository = setup(&pool).await?;
    tap(&pool, true).await?;
    let like = run_on_core(&repository).await?.remove(0);
    complete(&repository, &like).await?;

    late(&pool, "unlike_track", like.generation - 16).await?;
    let unlike = run_on_core(&repository).await?;
    assert_eq!(unlike.len(), 1);
    assert!(repository.drop_superseded(&unlike[0]).await?);

    assert!(queue(&pool).await?.is_empty());
    assert_eq!(mirror(&pool).await?, (true, false, true));
    assert_eq!(receipts(&pool, "sync_queue").await?, 2);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_like_older_than_the_purge_of_its_unlike_is_dropped_and_a_newer_one_runs(
    pool: PgPool,
) -> anyhow::Result<()> {
    let repository = setup(&pool).await?;
    tap(&pool, true).await?;
    let like = run_on_core(&repository).await?.remove(0);
    complete(&repository, &like).await?;
    tap(&pool, false).await?;
    let unlike = run_on_core(&repository).await?.remove(0);
    complete(&repository, &unlike).await?;
    let purged_v: i64 = sqlx::query_scalar(
        "SELECT wanted_v FROM user_likes_tracks
         WHERE public.scd_purge_intent(
             'user_likes_tracks',
             jsonb_build_object('user_id', user_id, 'sc_track_id', sc_track_id),
             wanted_v
         )",
    )
    .fetch_one(&pool)
    .await?;

    late(&pool, "like_track", like.generation).await?;
    let older = run_on_core(&repository).await?.remove(0);
    assert!(repository.drop_superseded(&older).await?);
    assert!(queue(&pool).await?.is_empty());

    late(&pool, "like_track", purged_v + 16).await?;
    let newer = run_on_core(&repository).await?.remove(0);
    assert!(!repository.drop_superseded(&newer).await?);
    complete(&repository, &newer).await?;
    assert!(queue(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_like_cancelled_by_a_lagging_node_below_its_attempt_is_dropped_and_the_unlike_runs(
    pool: PgPool,
) -> anyhow::Result<()> {
    let repository = setup(&pool).await?;
    tap(&pool, true).await?;
    let like = run_on_core(&repository).await?.remove(0);
    assert!(repository.record_remote_attempt(&like).await?);
    repository.release(&like).await?;

    regress(&pool, like.generation - 16, true).await?;

    assert_eq!(sent(&repository).await?, ["unlike_track"]);
    assert!(queue(&pool).await?.is_empty());
    assert_eq!(mirror(&pool).await?, (false, false, true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_like_retapped_by_a_lagging_node_below_its_attempt_is_still_sent(
    pool: PgPool,
) -> anyhow::Result<()> {
    let repository = setup(&pool).await?;
    tap(&pool, true).await?;
    let like = run_on_core(&repository).await?.remove(0);
    assert!(repository.record_remote_attempt(&like).await?);
    repository.release(&like).await?;

    regress(&pool, like.generation - 16, false).await?;

    assert_eq!(sent(&repository).await?, ["like_track"]);
    assert!(queue(&pool).await?.is_empty());
    assert_eq!(mirror(&pool).await?, (true, false, true));
    Ok(())
}

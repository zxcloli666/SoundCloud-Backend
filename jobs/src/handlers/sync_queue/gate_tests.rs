use serde_json::json;
use sqlx::PgPool;

use super::SyncQueueHandler;
use super::scope_tests::{Calls, attempt, setup};
use super::unclear_create_tests::run;

fn on_next_write(
    handler: &SyncQueueHandler,
    hook: impl FnOnce(&SyncQueueHandler) + Send + 'static,
) {
    *handler.before_write.lock().unwrap() = Some(Box::new(hook));
}

fn sent(calls: &Calls) -> Vec<String> {
    calls.lock().unwrap().clone()
}

async fn attempted(pool: &PgPool) -> anyhow::Result<(bool, bool, String)> {
    Ok(sqlx::query_as(
        "SELECT remote_attempted_generation IS NOT NULL, next_run_at > now(), coalesce(last_error, '')
         FROM sync_queue",
    )
    .fetch_one(pool)
    .await?)
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_write_admitted_before_its_action_was_paused_is_not_sent(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (app, handler, calls) = setup(&pool).await?;
    on_next_write(&handler, move |handler| {
        handler
            .write_pauses
            .pause_refused((app, "playlist_delete".to_owned()), 60);
    });

    let went = attempt(&pool, &handler, &calls, "21", "playlist_delete", "9").await?;

    assert!(!went);
    assert_eq!(
        attempted(&pool).await?,
        (
            false,
            true,
            "SoundCloud paused this action for the application".to_owned()
        )
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_write_admitted_before_its_account_was_paused_is_not_sent(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (_, handler, calls) = setup(&pool).await?;
    on_next_write(&handler, |handler| {
        handler
            .account_pauses
            .pause(("21".to_owned(), "like_track".to_owned()), 1800);
    });

    let went = attempt(&pool, &handler, &calls, "21", "like_track", "ok").await?;

    assert!(!went);
    assert_eq!(
        attempted(&pool).await?.2,
        "SoundCloud limits this action for the account"
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_create_stopped_before_sending_is_not_checked_as_unclear(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (app, handler, calls) = setup(&pool).await?;
    sqlx::query_file!(
        "../api/queries/sync_queue/service/enqueue.sql",
        "21",
        "playlist_create",
        "new:7d4c4c56-8c0f-4c2e-9a37-1b0e8a4f3e21",
        &json!({"playlist": {"title": "Mix", "sharing": "private"}}),
        false
    )
    .execute(&pool)
    .await?;
    on_next_write(&handler, move |handler| {
        handler
            .write_pauses
            .pause_refused((app, "playlist_create".to_owned()), 60);
    });

    run(&pool, &handler).await?;

    assert!(sent(&calls).is_empty());
    assert!(!attempted(&pool).await?.0);
    handler
        .write_pauses
        .succeeded(&(app, "playlist_create".to_owned()), None);

    run(&pool, &handler).await?;

    assert_eq!(sent(&calls), ["POST /playlists"]);
    Ok(())
}

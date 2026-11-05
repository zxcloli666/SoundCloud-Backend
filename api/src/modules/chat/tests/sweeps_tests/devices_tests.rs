use sqlx::PgPool;
use uuid::Uuid;

use super::support::{conversation, deltas, member, ok, state, user};
use crate::modules::chat::devices::lookup;
use crate::modules::chat::sweeps::devices;

async fn session(pg: &PgPool, user: &str) -> anyhow::Result<Uuid> {
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope)
         VALUES ($1, $2, $2, 'access', 'refresh', now() + interval '1 hour', '')",
    )
    .bind(connection)
    .bind(user)
    .execute(pg)
    .await?;
    let session = Uuid::now_v7();
    sqlx::query("INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)")
        .bind(session)
        .bind(connection)
        .execute(pg)
        .await?;
    Ok(session)
}

async fn device(pg: &PgPool, user: &str, session: Uuid, seen: &str) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chat.devices
            (id, user_id, session_hash, auth_pub, box_pub, platform, name, app_version, tier,
             last_seen_at)
         VALUES ($1, $2, sha256(uuid_send($3)), '\\x01'::bytea, '\\x02'::bytea, 'macos', 'Mac', '8.6.0', 'direct',
                 now() - $4::text::interval)",
    )
    .bind(id)
    .bind(user)
    .bind(session)
    .bind(seen)
    .execute(pg)
    .await?;
    Ok(id)
}

async fn status(pg: &PgPool, id: Uuid) -> anyhow::Result<(String, bool)> {
    Ok(
        sqlx::query_as("SELECT status, revoked_at IS NOT NULL FROM chat.devices WHERE id = $1")
            .bind(id)
            .fetch_one(pg)
            .await?,
    )
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn devices_whose_session_is_gone_for_an_hour_are_revoked(pg: PgPool) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let owner = user();
    let synthetic = format!("lt:{}", Uuid::now_v7().simple());
    let live = session(&pg, &owner).await?;
    let stale = device(&pg, &owner, Uuid::now_v7(), "2 hours").await?;
    let recent = device(&pg, &owner, Uuid::now_v7(), "10 minutes").await?;
    let signed_in = device(&pg, &owner, live, "2 hours").await?;
    let load_test = device(&pg, &synthetic, Uuid::now_v7(), "2 hours").await?;
    let group = conversation(&pg, "group", &owner).await?;
    member(&pg, group, &owner, "active").await?;

    let cached = ok(lookup(&chat, stale).await)?.expect("a device row");
    assert_eq!(cached.status, "active");

    assert_eq!(ok(devices::run_once(&chat).await)?, 1);

    assert!(chat.devices.row(stale).is_none(), "the cache forgets it");
    let fresh = ok(lookup(&chat, stale).await)?.expect("a device row");
    assert_eq!(fresh.status, "revoked");
    assert_eq!(status(&pg, stale).await?, ("revoked".to_owned(), true));
    for kept in [recent, signed_in, load_test] {
        assert_eq!(status(&pg, kept).await?, ("active".to_owned(), false));
    }
    let rekey: bool = sqlx::query_scalar("SELECT rekey FROM chat.conversations WHERE id = $1")
        .bind(group)
        .fetch_one(&pg)
        .await?;
    assert!(
        rekey,
        "the next send in every chat of the owner starts a new epoch"
    );
    assert!(deltas(&chat, &owner).await?.contains(&"devices".to_owned()));
    assert_eq!(ok(devices::run_once(&chat).await)?, 0);
    Ok(())
}

async fn idle_devices(pg: &PgPool, user: &str, count: i64) -> anyhow::Result<()> {
    sqlx::query(
        "WITH live AS (
             INSERT INTO sessions (id) SELECT gen_random_uuid() FROM generate_series(1, $2)
             RETURNING id
         )
         INSERT INTO chat.devices
             (id, user_id, session_hash, auth_pub, box_pub, platform, name, app_version, tier,
              last_seen_at)
         SELECT gen_random_uuid(), $1, sha256(uuid_send(live.id)), '\\x01'::bytea, '\\x02'::bytea,
                'linux', 'idle', '8.6.0', 'direct', now() - interval '3 hours'
         FROM live",
    )
    .bind(user)
    .bind(count)
    .execute(pg)
    .await?;
    Ok(())
}

async fn unchecked(pg: &PgPool, user: &str) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM chat.devices
         WHERE user_id = $1 AND status = 'active' AND alive_checked_at IS NULL",
    )
    .bind(user)
    .fetch_one(pg)
    .await?)
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn fifty_thousand_idle_devices_lose_their_dead_sessions_in_one_run(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let owner = user();
    idle_devices(&pg, &owner, 50_000).await?;
    let mut dead = Vec::new();
    for _ in 0..25 {
        dead.push(device(&pg, &owner, Uuid::new_v4(), "2 hours").await?);
    }

    let revoked = ok(devices::run_once(&chat).await)?;

    assert_eq!(revoked, 25);
    for id in dead {
        assert_eq!(status(&pg, id).await?, ("revoked".to_owned(), true));
    }
    assert_eq!(unchecked(&pg, &owner).await?, 0);
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chat.devices WHERE user_id = $1 AND status = 'active'",
    )
    .bind(&owner)
    .fetch_one(&pg)
    .await?;
    assert_eq!(active, 50_000);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_failed_batch_is_checked_again_on_the_next_run(pg: PgPool) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let owner = user();
    idle_devices(&pg, &owner, 2_500).await?;
    let dead = device(&pg, &owner, Uuid::new_v4(), "2 hours").await?;
    sqlx::raw_sql(
        "CREATE FUNCTION public.sweep_fault() RETURNS trigger LANGUAGE plpgsql
         SET search_path = pg_catalog, public, pg_temp
         AS $f$ BEGIN RAISE EXCEPTION 'sweep batch failed'; END $f$;
         CREATE TRIGGER sweep_fault BEFORE UPDATE OF status ON chat.devices
         FOR EACH ROW WHEN (NEW.status = 'revoked') EXECUTE FUNCTION public.sweep_fault();",
    )
    .execute(&pg)
    .await?;

    let failed = devices::run_once(&chat).await;
    let after_failure = status(&pg, dead).await?;
    let dead_unchecked: bool =
        sqlx::query_scalar("SELECT alive_checked_at IS NULL FROM chat.devices WHERE id = $1")
            .bind(dead)
            .fetch_one(&pg)
            .await?;
    sqlx::raw_sql(
        "DROP TRIGGER sweep_fault ON chat.devices;
         DROP FUNCTION public.sweep_fault();",
    )
    .execute(&pg)
    .await?;
    let retried = ok(devices::run_once(&chat).await)?;

    assert!(failed.is_err(), "the faulty batch fails the run");
    assert_eq!(after_failure, ("active".to_owned(), false));
    assert!(
        dead_unchecked,
        "a rolled back batch is not marked as checked"
    );
    assert_eq!(retried, 1);
    assert_eq!(status(&pg, dead).await?, ("revoked".to_owned(), true));
    assert_eq!(unchecked(&pg, &owner).await?, 0);
    Ok(())
}

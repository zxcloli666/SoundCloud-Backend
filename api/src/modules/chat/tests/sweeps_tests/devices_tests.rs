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
            (id, user_id, session_id, auth_pub, box_pub, platform, name, app_version, tier,
             last_seen_at)
         VALUES ($1, $2, $3, '\\x01'::bytea, '\\x02'::bytea, 'macos', 'Mac', '8.6.0', 'direct',
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

#[sqlx::test(migrations = "./migrations")]
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

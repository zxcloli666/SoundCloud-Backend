use sqlx::PgPool;

use super::*;

fn claimed() -> Profile {
    Profile {
        name: "Somebody Famous".to_owned(),
        avatar_url: Some("https://i1.sndcdn.com/claimed.jpg".to_owned()),
    }
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_stored_account_wins_over_what_the_client_claims(pool: PgPool) -> anyhow::Result<()> {
    let unknown = account_profile(&pool, "41", Uuid::now_v7(), claimed()).await?;
    assert_eq!(unknown.name, "Somebody Famous");
    assert_eq!(
        unknown.avatar_url.as_deref(),
        Some("https://i1.sndcdn.com/claimed.jpg")
    );

    sqlx::query(
        "INSERT INTO users (sc_user_id, urn, username, username_normalized, avatar_url)
         VALUES ('42', 'soundcloud:users:42', 'Real Name', 'real name',
                 'https://i1.sndcdn.com/real.jpg')",
    )
    .execute(&pool)
    .await?;
    let known = account_profile(&pool, "42", Uuid::now_v7(), claimed()).await?;
    assert_eq!(known.name, "Real Name");
    assert_eq!(
        known.avatar_url.as_deref(),
        Some("https://i1.sndcdn.com/real.jpg")
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_connection_name_fills_in_for_a_missing_account(pool: PgPool) -> anyhow::Result<()> {
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope)
         VALUES ($1, '43', 'Connected Name', 'access', 'refresh', now() + interval '1 hour', '')",
    )
    .bind(connection)
    .execute(&pool)
    .await?;
    let session = Uuid::now_v7();
    sqlx::query("INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)")
        .bind(session)
        .bind(connection)
        .execute(&pool)
        .await?;

    let profile = account_profile(&pool, "43", session, claimed()).await?;
    assert_eq!(profile.name, "Connected Name");
    let stranger = account_profile(&pool, "43", Uuid::now_v7(), claimed()).await?;
    assert_eq!(stranger.name, "Somebody Famous");
    Ok(())
}

use sqlx::PgPool;

use super::*;

fn claimed() -> Profile {
    Profile {
        name: "Somebody Famous".to_owned(),
        avatar_url: Some("https://i1.sndcdn.com/claimed.jpg".to_owned()),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn the_stored_account_wins_over_what_the_client_claims(pool: PgPool) -> anyhow::Result<()> {
    let unknown = account_profile(&pool, "41", claimed()).await?;
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
    let known = account_profile(&pool, "42", claimed()).await?;
    assert_eq!(known.name, "Real Name");
    assert_eq!(
        known.avatar_url.as_deref(),
        Some("https://i1.sndcdn.com/real.jpg")
    );
    Ok(())
}

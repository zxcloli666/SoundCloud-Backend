use super::*;

const UPLOAD_WITH_NUL: &str = r#"{
    "urn": "soundcloud:tracks:42",
    "title": "Night\u0000 Drive",
    "description": "first line\u0000second line",
    "genre": "Synth\u0000wave",
    "tag_list": "retro\u0000 synth",
    "label_name": "Neon\u0000 Records",
    "duration": 120000,
    "user": {
        "urn": "soundcloud:users:17",
        "username": "Driver\u0000",
        "avatar_url": "https://i1.sndcdn.com/avatars-17\u0000-large.jpg"
    }
}"#;

async fn handler(pool: &PgPool) -> anyhow::Result<AccountWalkHandler> {
    crate::db::migrations::run_core(pool, None).await?;
    let reader = Arc::new(PublicCatalogReader::offline(pool.clone()));
    let config = AccountWalkConfig {
        walk_interval_days: 7,
        lease_seconds: 60,
        batch: 1,
        concurrency: 1,
    };
    Ok(AccountWalkHandler::new(pool.clone(), reader, config))
}

#[sqlx::test(migrations = false)]
async fn account_walk_stores_an_upload_with_nul_text_without_the_nul(
    pool: PgPool,
) -> anyhow::Result<()> {
    let handler = handler(&pool).await?;
    let upload: Value = serde_json::from_str(UPLOAD_WITH_NUL)?;
    let fields =
        ScTrackFields::from_sc(&upload).ok_or_else(|| anyhow::anyhow!("invalid upload"))?;

    handler
        .ingest(&fields, Observation::begin(&pool).await?)
        .await?;

    let stored: (String, String, String, Vec<String>, String, String) = sqlx::query_as(
        "SELECT title, description, genre, tags, uploader_username, sc_metadata->>'label_name'
         FROM tracks WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        stored,
        (
            "Night Drive".into(),
            "first linesecond line".into(),
            "Synthwave".into(),
            vec!["retro".into(), "synth".into()],
            "Driver".into(),
            "Neon Records".into(),
        )
    );
    assert_eq!(
        avatar_of(&fields).as_deref(),
        Some("https://i1.sndcdn.com/avatars-17-t500x500.jpg")
    );
    Ok(())
}

async fn link_account(
    pool: &PgPool,
    artist_id: Uuid,
    sc_user_id: &str,
    verified: bool,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO artist_sc_accounts (artist_id, sc_user_id, role, source, verified)
         VALUES ($1, $2, 'main', 'mb_resolve', $3)",
    )
    .bind(artist_id)
    .bind(sc_user_id)
    .bind(verified)
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn account_walk_drops_a_gone_unverified_account_and_keeps_a_verified_one(
    pool: PgPool,
) -> anyhow::Result<()> {
    let handler = handler(&pool).await?;
    let artist_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source)
         VALUES ($1, 'Gone Artist', 'gone artist', 'sc_verified')",
    )
    .bind(artist_id)
    .execute(&pool)
    .await?;
    link_account(&pool, artist_id, "404", false).await?;
    link_account(&pool, artist_id, "410", true).await?;

    handler.forget_gone_account(artist_id, "404").await?;
    handler.forget_gone_account(artist_id, "410").await?;

    let left: Vec<String> = sqlx::query_scalar(
        "SELECT sc_user_id FROM artist_sc_accounts WHERE artist_id = $1 ORDER BY sc_user_id",
    )
    .bind(artist_id)
    .fetch_all(&pool)
    .await?;
    assert_eq!(left, vec!["410".to_owned()]);
    Ok(())
}

use sqlx::PgPool;

async fn install(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(include_str!("../../../migrations/0057_background_jobs.sql"))
        .execute(pool)
        .await?;
    sqlx::raw_sql(
        "CREATE TABLE featured_items (
            id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
            type text NOT NULL, sc_urn text NOT NULL,
            weight integer NOT NULL DEFAULT 1, active boolean NOT NULL DEFAULT true,
            created_at timestamp NOT NULL DEFAULT now()
        );
        CREATE TABLE tracks (urn text PRIMARY KEY, sharing text NOT NULL, deleted_at timestamp);
        CREATE TABLE playlists (urn text PRIMARY KEY, sharing text NOT NULL, deleted_at timestamp);
        CREATE TABLE users (urn text PRIMARY KEY);",
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn featured_changes_atomically_enqueue_public_catalog_hydration(
    pool: PgPool,
) -> anyhow::Result<()> {
    install(&pool).await?;
    let service = super::service::FeaturedService::new(pool.clone());
    assert!(service.create("track", "42", None, None).await.is_err());
    let item = service
        .create("track", "soundcloud:tracks:42", None, None)
        .await?;
    service
        .update(&item.id.to_string(), None, None, Some(2), None)
        .await?;
    let jobs: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT dedup_key, payload FROM background_jobs WHERE kind = 'catalog.refresh'",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(jobs.len(), 1);
    let (key, body) = jobs
        .first()
        .ok_or_else(|| anyhow::anyhow!("hydration job missing"))?;
    assert_eq!(key, "track:42:public");
    let payload: backend_contracts::Versioned<backend_contracts::CatalogRefreshPayload> =
        serde_json::from_value(body.clone())?;
    let backend_contracts::Versioned::V1(payload) = payload;
    assert!(payload.owner_id.is_none());
    assert!(
        service
            .update(&item.id.to_string(), Some("user"), None, None, None)
            .await
            .is_err()
    );
    let unchanged: String = sqlx::query_scalar("SELECT type FROM featured_items WHERE id = $1")
        .bind(item.id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(unchanged, "track");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn hydration_failure_rolls_back_featured_creation(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    sqlx::query("ALTER TABLE background_jobs ADD CHECK (kind <> 'catalog.refresh')")
        .execute(&pool)
        .await?;
    let service = super::service::FeaturedService::new(pool.clone());
    assert!(
        service
            .create("track", "soundcloud:tracks:42", None, None)
            .await
            .is_err()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM featured_items")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn selection_excludes_missing_private_and_inactive_entities(
    pool: PgPool,
) -> anyhow::Result<()> {
    install(&pool).await?;
    sqlx::raw_sql(
        "INSERT INTO tracks (urn, sharing) VALUES ('private-track', 'private'), ('inactive-track', 'public');
         INSERT INTO playlists (urn, sharing) VALUES ('private-playlist', 'private');
         INSERT INTO users VALUES ('visible-user');
         INSERT INTO featured_items (type, sc_urn, active) VALUES
             ('track', 'missing-track', true), ('track', 'private-track', true),
             ('track', 'inactive-track', false), ('playlist', 'private-playlist', true),
             ('playlist', 'missing-playlist', true), ('user', 'missing-user', true),
             ('user', 'visible-user', true);",
    )
    .execute(&pool)
    .await?;

    let rows = sqlx::query_file!("queries/featured/service/pick_active.sql")
        .fetch_all(&pool)
        .await?;

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows.first().map(|row| row.sc_urn.as_str()),
        Some("visible-user")
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn selection_excludes_deleted_entities_even_with_public_metadata(
    pool: PgPool,
) -> anyhow::Result<()> {
    install(&pool).await?;
    sqlx::raw_sql(
        "INSERT INTO playlists VALUES ('deleted-playlist', 'public', now());
         INSERT INTO tracks VALUES ('deleted-track', 'public', now());
         INSERT INTO featured_items (type, sc_urn) VALUES
             ('playlist', 'deleted-playlist'), ('track', 'deleted-track');",
    )
    .execute(&pool)
    .await?;
    let rows = sqlx::query_file!("queries/featured/service/pick_active.sql")
        .fetch_all(&pool)
        .await?;
    assert!(rows.is_empty());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn selection_is_empty_when_no_local_public_entity_exists(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    sqlx::raw_sql("INSERT INTO featured_items (type, sc_urn) VALUES ('track', 'missing');")
        .execute(&pool)
        .await?;

    let rows = sqlx::query_file!("queries/featured/service/pick_active.sql")
        .fetch_all(&pool)
        .await?;

    assert!(rows.is_empty());
    Ok(())
}

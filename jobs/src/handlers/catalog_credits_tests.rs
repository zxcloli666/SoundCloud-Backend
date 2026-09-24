use sqlx::PgPool;
use uuid::Uuid;

use super::catalog_credits::CatalogCreditHandler;

async fn install(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE artists (
             id uuid PRIMARY KEY,
             name text NOT NULL
         );
         CREATE TABLE tracks (
             id uuid PRIMARY KEY,
             sc_track_id text NOT NULL,
             uploader_sc_user_id text
         );
         CREATE TABLE artist_sc_accounts (
             artist_id uuid NOT NULL,
             sc_user_id text NOT NULL,
             source varchar(16) NOT NULL,
             verified boolean NOT NULL DEFAULT false,
             PRIMARY KEY (artist_id, sc_user_id)
         );
         CREATE TABLE track_artists (
             track_id uuid NOT NULL,
             artist_id uuid NOT NULL,
             role varchar(16) NOT NULL,
             position smallint NOT NULL DEFAULT 0,
             source varchar(16) NOT NULL,
             confidence real NOT NULL DEFAULT 0,
             evidence varchar(24) NOT NULL DEFAULT 'unattributed',
             PRIMARY KEY (track_id, artist_id, role)
         );",
    )
    .execute(pool)
    .await?;
    Ok(())
}

struct Fixture {
    track: Uuid,
    artist: Uuid,
}

async fn seed(
    pool: &PgPool,
    uploader: Option<&str>,
    evidence: &str,
    identity: Option<(&str, bool, &str)>,
) -> anyhow::Result<Fixture> {
    let track = Uuid::now_v7();
    let artist = Uuid::now_v7();
    sqlx::query("INSERT INTO artists VALUES ($1, 'artist')")
        .bind(artist)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO tracks VALUES ($1, '1', $2)")
        .bind(track)
        .bind(uploader)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO track_artists (track_id, artist_id, role, source, confidence, evidence)
         VALUES ($1, $2, 'primary', 'heuristic', 0.25, $3)",
    )
    .bind(track)
    .bind(artist)
    .bind(evidence)
    .execute(pool)
    .await?;
    if let Some((sc_user_id, verified, source)) = identity {
        sqlx::query("INSERT INTO artist_sc_accounts VALUES ($1, $2, $3, $4)")
            .bind(artist)
            .bind(sc_user_id)
            .bind(source)
            .bind(verified)
            .execute(pool)
            .await?;
    }
    Ok(Fixture { track, artist })
}

async fn credit(pool: &PgPool, at: &Fixture) -> anyhow::Result<(String, f32, String)> {
    let row = sqlx::query_as::<_, (String, f32, String)>(
        "SELECT source, confidence, evidence FROM track_artists
         WHERE track_id = $1 AND artist_id = $2",
    )
    .bind(at.track)
    .bind(at.artist)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

#[sqlx::test(migrations = false)]
async fn a_verified_uploader_identity_confirms_a_weak_credit(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    let at = seed(
        &pool,
        Some("42"),
        "uploader_name",
        Some(("42", true, "manual")),
    )
    .await?;

    CatalogCreditHandler::new(pool.clone()).review().await?;

    assert_eq!(
        credit(&pool, &at).await?,
        (
            "sc_verified".to_owned(),
            0.95,
            "verified_account".to_owned()
        )
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_mb_resolved_identity_counts_as_confirmation(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    let at = seed(
        &pool,
        Some("42"),
        "title_heuristic",
        Some(("42", false, "mb_resolve")),
    )
    .await?;

    CatalogCreditHandler::new(pool.clone()).review().await?;

    assert_eq!(credit(&pool, &at).await?.2, "verified_account".to_owned());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unverified_account_never_confirms_a_credit(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    let at = seed(
        &pool,
        Some("42"),
        "uploader_name",
        Some(("42", false, "walker")),
    )
    .await?;

    CatalogCreditHandler::new(pool.clone()).review().await?;

    assert_eq!(credit(&pool, &at).await?.2, "uploader_name".to_owned());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_identity_for_a_different_uploader_never_confirms(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    let at = seed(
        &pool,
        Some("42"),
        "uploader_name",
        Some(("99", true, "manual")),
    )
    .await?;

    CatalogCreditHandler::new(pool.clone()).review().await?;

    assert_eq!(credit(&pool, &at).await?.2, "uploader_name".to_owned());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_track_without_an_uploader_is_never_confirmed(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    let at = seed(&pool, None, "unattributed", Some(("42", true, "manual"))).await?;

    CatalogCreditHandler::new(pool.clone()).review().await?;

    assert_eq!(credit(&pool, &at).await?.2, "unattributed".to_owned());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_strong_credit_is_left_alone(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    let at = seed(
        &pool,
        Some("42"),
        "external_id",
        Some(("42", true, "manual")),
    )
    .await?;
    sqlx::query("UPDATE track_artists SET source = 'mb', confidence = 0.9")
        .execute(&pool)
        .await?;

    CatalogCreditHandler::new(pool.clone()).review().await?;

    assert_eq!(
        credit(&pool, &at).await?,
        ("mb".to_owned(), 0.9, "external_id".to_owned())
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn confirmation_is_idempotent(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    let at = seed(
        &pool,
        Some("42"),
        "ai_inference",
        Some(("42", true, "manual")),
    )
    .await?;
    let handler = CatalogCreditHandler::new(pool.clone());

    handler.review().await?;
    handler.review().await?;

    assert_eq!(credit(&pool, &at).await?.2, "verified_account".to_owned());
    Ok(())
}

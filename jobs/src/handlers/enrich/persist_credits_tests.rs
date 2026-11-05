use sqlx::PgPool;
use uuid::Uuid;

use super::apply;
use crate::handlers::enrich::resolver::{
    ArtistCandidate, CreditEvidence, ResolveResult, ResolveSource,
};

type Version = (String, String, String, String);

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn re_enriching_an_unchanged_track_writes_no_credit(pool: PgPool) -> anyhow::Result<()> {
    let track = insert_track(&pool).await?;
    let credits = resolved(&["Northern Lights"], &["Velvet Harbor", "Quiet Engines"]);

    apply(&pool, track, &credits, None, None).await?;
    let before = versions(&pool, track).await?;
    apply(&pool, track, &credits, None, None).await?;

    assert_eq!(before.len(), 3);
    assert_eq!(versions(&pool, track).await?, before);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_changed_credit_list_rewrites_only_the_credits_that_changed(
    pool: PgPool,
) -> anyhow::Result<()> {
    let track = insert_track(&pool).await?;

    apply(
        &pool,
        track,
        &resolved(&["Northern Lights"], &["Velvet Harbor"]),
        None,
        None,
    )
    .await?;
    let before = versions(&pool, track).await?;
    apply(
        &pool,
        track,
        &resolved(&["Northern Lights"], &["Quiet Engines"]),
        None,
        None,
    )
    .await?;
    let after = versions(&pool, track).await?;

    let roles: Vec<(&str, &str)> = after
        .iter()
        .map(|(name, role, _, _)| (name.as_str(), role.as_str()))
        .collect();
    assert_eq!(
        roles,
        [
            ("Northern Lights", "primary"),
            ("Quiet Engines", "featured")
        ]
    );
    assert_eq!(after.first(), before.first());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_credit_blocked_after_enrich_is_dropped_on_the_next_enrich(
    pool: PgPool,
) -> anyhow::Result<()> {
    let track = insert_track(&pool).await?;
    let credits = resolved(&["Northern Lights"], &["Velvet Harbor"]);

    apply(&pool, track, &credits, None, None).await?;
    let before = versions(&pool, track).await?;
    sqlx::query(
        "INSERT INTO track_artist_blocks (track_id, artist_id)
         SELECT credit.track_id, credit.artist_id
         FROM track_artists credit
                  JOIN artists a ON a.id = credit.artist_id
         WHERE credit.track_id = $1 AND a.name = 'Velvet Harbor'",
    )
    .bind(track)
    .execute(&pool)
    .await?;
    apply(&pool, track, &credits, None, None).await?;
    let after = versions(&pool, track).await?;

    assert_eq!(after.len(), 1);
    assert_eq!(after.first(), before.first());
    Ok(())
}

fn resolved(primary: &[&str], featured: &[&str]) -> ResolveResult {
    ResolveResult {
        source: ResolveSource::Genius,
        confidence: 0.85,
        primary: primary.iter().copied().map(credit).collect(),
        featured: featured.iter().copied().map(credit).collect(),
        ..Default::default()
    }
}

fn credit(name: &str) -> ArtistCandidate {
    ArtistCandidate {
        name: name.to_owned(),
        genius_id: Some(name.to_lowercase().replace(' ', "-")),
        ..Default::default()
    }
    .attributed(ResolveSource::Genius, 0.85, CreditEvidence::ExternalId)
}

async fn insert_track(pool: &PgPool) -> anyhow::Result<Uuid> {
    let track: Uuid = sqlx::query_scalar(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms)
         VALUES ($1, 'soundcloud:tracks:' || $1, 'Night Drive', 'night drive', 180000)
         RETURNING id",
    )
    .bind(Uuid::new_v4().simple().to_string())
    .fetch_one(pool)
    .await?;
    Ok(track)
}

async fn versions(pool: &PgPool, track: Uuid) -> anyhow::Result<Vec<Version>> {
    let rows = sqlx::query_as(
        "SELECT a.name, credit.role::text, credit.ctid::text, credit.xmin::text
         FROM track_artists credit
                  JOIN artists a ON a.id = credit.artist_id
         WHERE credit.track_id = $1
         ORDER BY credit.role DESC, a.name",
    )
    .bind(track)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

use std::collections::HashSet;

use catalog_normalize::normalize_name;
use sqlx::PgPool;
use uuid::Uuid;

use super::apply;
use crate::handlers::enrich::resolver::{
    ArtistCandidate, CreditEvidence, LocalSignals, ResolveResult, ResolveSource, TrackContext,
};

const Z2_HOME: &str = "1211580634";
const Z2_AVATAR: &str = "https://i1.sndcdn.com/avatars-0nqZTr2cOae7fdHx-z56c8Q-t500x500.jpg";

struct Uploader(&'static str, &'static str);

async fn seed_artist(pool: &PgPool, name: &str) -> anyhow::Result<Uuid> {
    let id = sqlx::query_scalar(
        "INSERT INTO artists (name, normalized_name, source) VALUES ($1, $2, 'mb') RETURNING id",
    )
    .bind(name)
    .bind(normalize_name(name))
    .fetch_one(pool)
    .await?;
    Ok(id)
}

async fn seed_account(
    pool: &PgPool,
    artist_id: Uuid,
    sc_user_id: &str,
    source: &str,
    verified: bool,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO artist_sc_accounts (artist_id, sc_user_id, role, source, verified)
         VALUES ($1, $2, 'main', $3, $4)",
    )
    .bind(artist_id)
    .bind(sc_user_id)
    .bind(source)
    .bind(verified)
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_z2(pool: &PgPool) -> anyhow::Result<Uuid> {
    let z2 = seed_artist(pool, "z²").await?;
    seed_account(pool, z2, Z2_HOME, "mb_resolve", false).await?;
    sqlx::query("UPDATE artists SET sc_user_id = $2, avatar_url = $3 WHERE id = $1")
        .bind(z2)
        .bind(Z2_HOME)
        .bind(Z2_AVATAR)
        .execute(pool)
        .await?;
    Ok(z2)
}

fn heuristic(title: &str, uploader: &Uploader) -> ResolveResult {
    let Uploader(id, name) = *uploader;
    let context = TrackContext {
        title: title.to_owned(),
        uploader_username: Some(name.to_owned()),
        uploader_sc_user_id: Some(id.to_owned()),
        duration_ms: Some(180_000),
        isrc: None,
        metadata_artist: None,
        description: None,
    };
    LocalSignals::build_with_dictionary(&context, &HashSet::new()).heuristic(&context)
}

async fn enrich(
    pool: &PgPool,
    title: &str,
    uploader: &Uploader,
    resolved: &ResolveResult,
) -> anyhow::Result<Uuid> {
    let Uploader(id, name) = *uploader;
    let track_id: Uuid = sqlx::query_scalar(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms,
                             uploader_sc_user_id, uploader_username)
         VALUES ($1, 'soundcloud:tracks:' || $1, $2, lower($2), 180000, $3, $4)
         RETURNING id",
    )
    .bind(Uuid::new_v4().simple().to_string())
    .bind(title)
    .bind(id)
    .bind(name)
    .fetch_one(pool)
    .await?;
    apply(pool, track_id, resolved, Some(id), Some(name)).await?;
    Ok(track_id)
}

async fn roles_of(pool: &PgPool, track_id: Uuid, artist_id: Uuid) -> anyhow::Result<Vec<String>> {
    let roles = sqlx::query_scalar(
        "SELECT role FROM track_artists WHERE track_id = $1 AND artist_id = $2 ORDER BY role",
    )
    .bind(track_id)
    .bind(artist_id)
    .fetch_all(pool)
    .await?;
    Ok(roles)
}

async fn primary_of(pool: &PgPool, track_id: Uuid) -> anyhow::Result<(Option<Uuid>, String)> {
    let row = sqlx::query_as("SELECT primary_artist_id, upload_kind FROM tracks WHERE id = $1")
        .bind(track_id)
        .fetch_one(pool)
        .await?;
    Ok(row)
}

async fn accounts_of(
    pool: &PgPool,
    artist_id: Uuid,
) -> anyhow::Result<Vec<(String, String, String)>> {
    let accounts = sqlx::query_as(
        "SELECT sc_user_id, role::text, source::text FROM artist_sc_accounts
         WHERE artist_id = $1 ORDER BY sc_user_id",
    )
    .bind(artist_id)
    .fetch_all(pool)
    .await?;
    Ok(accounts)
}

async fn link_of(
    pool: &PgPool,
    artist_id: Uuid,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    let row = sqlx::query_as("SELECT sc_user_id, avatar_url FROM artists WHERE id = $1")
        .bind(artist_id)
        .fetch_one(pool)
        .await?;
    Ok(row)
}

#[sqlx::test(migrations = "../api/migrations")]
async fn namesake_uploaders_stay_off_a_short_named_artist(pool: PgPool) -> anyhow::Result<()> {
    let z2 = seed_z2(&pool).await?;
    let namesakes = [
        Uploader("1281291292", "z2"),
        Uploader("262191304", "Z2"),
        Uploader("972358267", "z²"),
        Uploader("1384256286", "ᶻ²"),
    ];

    for uploader in &namesakes {
        let resolved = heuristic("night drive", uploader);
        assert_eq!(normalize_name(uploader.1), "z2");
        let track = enrich(&pool, "night drive", uploader, &resolved).await?;

        assert!(roles_of(&pool, track, z2).await?.is_empty());
        assert_eq!(primary_of(&pool, track).await?.0, None);
    }

    assert_eq!(
        accounts_of(&pool, z2).await?,
        [(
            Z2_HOME.to_owned(),
            "main".to_owned(),
            "mb_resolve".to_owned()
        )]
    );
    assert_eq!(
        link_of(&pool, z2).await?,
        (Some(Z2_HOME.to_owned()), Some(Z2_AVATAR.to_owned()))
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn the_artists_own_account_keeps_its_uploads(pool: PgPool) -> anyhow::Result<()> {
    let z2 = seed_z2(&pool).await?;
    let home = Uploader(Z2_HOME, "z²");

    let track = enrich(
        &pool,
        "night drive",
        &home,
        &heuristic("night drive", &home),
    )
    .await?;

    assert_eq!(roles_of(&pool, track, z2).await?, ["primary"]);
    assert_eq!(
        primary_of(&pool, track).await?,
        (Some(z2), "original".to_owned())
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_short_name_in_a_strangers_title_is_not_a_primary_but_a_feature_stays(
    pool: PgPool,
) -> anyhow::Result<()> {
    let z2 = seed_z2(&pool).await?;
    let oyg = Uploader("1280256208", "oyg");

    let claimed = enrich(&pool, "z² - night", &oyg, &heuristic("z² - night", &oyg)).await?;
    let featured = "oyg - night (feat. z²)";
    let feature = enrich(&pool, featured, &oyg, &heuristic(featured, &oyg)).await?;

    assert!(roles_of(&pool, claimed, z2).await?.is_empty());
    assert_eq!(roles_of(&pool, feature, z2).await?, ["featured"]);
    assert_ne!(primary_of(&pool, feature).await?.0, Some(z2));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn an_external_credit_still_reaches_a_short_named_artist(pool: PgPool) -> anyhow::Result<()> {
    let z2 = seed_z2(&pool).await?;
    let stranger = Uploader("1072135747", "owari");
    let resolved = ResolveResult {
        source: ResolveSource::Genius,
        confidence: 0.85,
        primary: vec![
            ArtistCandidate {
                name: "z²".to_owned(),
                genius_id: Some("3184042".to_owned()),
                ..Default::default()
            }
            .attributed(ResolveSource::Genius, 0.85, CreditEvidence::ExternalId),
        ],
        ..Default::default()
    };

    let track = enrich(&pool, "z² - night", &stranger, &resolved).await?;

    assert_eq!(roles_of(&pool, track, z2).await?, ["primary"]);
    assert_eq!(
        primary_of(&pool, track).await?,
        (Some(z2), "reupload".to_owned())
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_namesake_account_of_a_known_artist_is_left_out_but_reuploads_are_not(
    pool: PgPool,
) -> anyhow::Result<()> {
    let dekma = seed_artist(&pool, "dekma").await?;
    seed_account(&pool, dekma, "500", "manual", true).await?;
    let namesake = Uploader("501", "dekma");
    let fan = Uploader("502", "slowed songs");

    let own_name = enrich(
        &pool,
        "без шансов",
        &namesake,
        &heuristic("без шансов", &namesake),
    )
    .await?;
    let titled = "dekma - без шансов";
    let reupload = enrich(&pool, titled, &fan, &heuristic(titled, &fan)).await?;

    assert!(roles_of(&pool, own_name, dekma).await?.is_empty());
    assert_eq!(roles_of(&pool, reupload, dekma).await?, ["primary"]);
    assert_eq!(
        accounts_of(&pool, dekma).await?,
        [("500".to_owned(), "main".to_owned(), "manual".to_owned())]
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_name_match_gives_an_alt_account_and_never_the_artist_link(
    pool: PgPool,
) -> anyhow::Result<()> {
    let uploader = Uploader("600", "Monarchy");

    let track = enrich(
        &pool,
        "night drive",
        &uploader,
        &heuristic("night drive", &uploader),
    )
    .await?;

    let (Some(artist), kind) = primary_of(&pool, track).await? else {
        anyhow::bail!("the uploader name should still credit a new artist");
    };
    assert_eq!(kind, "unknown");
    assert_eq!(
        accounts_of(&pool, artist).await?,
        [("600".to_owned(), "alt".to_owned(), "auto_match".to_owned())]
    );
    assert_eq!(link_of(&pool, artist).await?, (None, None));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_short_name_never_gets_an_auto_match_account(pool: PgPool) -> anyhow::Result<()> {
    let uploader = Uploader("700", "oyg");

    let track = enrich(
        &pool,
        "night drive",
        &uploader,
        &heuristic("night drive", &uploader),
    )
    .await?;

    let (Some(artist), _) = primary_of(&pool, track).await? else {
        anyhow::bail!("an artist without accounts still takes the upload");
    };
    assert!(accounts_of(&pool, artist).await?.is_empty());
    assert_eq!(link_of(&pool, artist).await?, (None, None));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_trusted_account_stops_new_auto_matches(pool: PgPool) -> anyhow::Result<()> {
    let monarchy = seed_artist(&pool, "Monarchy").await?;
    seed_account(&pool, monarchy, "800", "mb_resolve", false).await?;
    let fan = Uploader("801", "Monarchy Archive");

    let titled = "Monarchy - night drive";
    let track = enrich(&pool, titled, &fan, &heuristic(titled, &fan)).await?;

    assert_eq!(roles_of(&pool, track, monarchy).await?, ["primary"]);
    assert_eq!(
        accounts_of(&pool, monarchy).await?,
        [("800".to_owned(), "main".to_owned(), "mb_resolve".to_owned())]
    );
    Ok(())
}

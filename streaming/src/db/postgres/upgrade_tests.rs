use tokio_postgres::error::SqlState;
use tokio_postgres::{Client, GenericClient, NoTls};

use super::{NEXT_REST, NEXT_STAR_LIKED, PICK_LIKED, PICK_REST, PICK_STAR_LIKED, STAR_SCAN_WINDOW};

const COOLDOWN: f64 = 6.0 * 60.0 * 60.0;

async fn connect() -> Result<Client, Box<dyn std::error::Error>> {
    let database_url = std::env::var("STREAMING_SCHEMA_TEST_DATABASE_URL")?;
    let (client, connection) = tokio_postgres::connect(&database_url, NoTls).await?;
    tokio::spawn(connection);
    Ok(client)
}

async fn only_these_tracks_wait(
    client: &impl GenericClient,
    seed: &str,
) -> Result<(), tokio_postgres::Error> {
    client
        .batch_execute(&format!(
            "SELECT pg_advisory_xact_lock(hashtext('streaming upgrade tests'));
             DELETE FROM star_track_tiers;
             UPDATE tracks SET hq_upgrade_pending = false WHERE hq_upgrade_pending;
             {seed}"
        ))
        .await
}

async fn pick(
    client: &impl GenericClient,
    statement: &str,
    limit: i64,
) -> Result<Vec<String>, tokio_postgres::Error> {
    let rows = client.query(statement, &[&limit, &COOLDOWN, &"sq"]).await?;
    Ok(urns(&rows))
}

async fn pick_star_liked(
    client: &impl GenericClient,
    limit: i64,
) -> Result<Vec<String>, tokio_postgres::Error> {
    let rows = client
        .query(
            PICK_STAR_LIKED,
            &[&limit, &COOLDOWN, &"sq", &STAR_SCAN_WINDOW],
        )
        .await?;
    Ok(urns(&rows))
}

fn urns(rows: &[tokio_postgres::Row]) -> Vec<String> {
    let mut urns: Vec<String> = rows.iter().map(|row| row.get(0)).collect();
    urns.sort();
    urns
}

const STAR_LIKES: &str =
    "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms,
                         storage_quality, hq_upgrade_pending, hq_upgrade_last_at, index_priority)
     VALUES ('upgrade-test-1', 'soundcloud:tracks:upgrade-test-1', 'Tried', 'tried', 1000,
             'sq', true, now() - interval '7 hours', 5),
            ('upgrade-test-2', 'soundcloud:tracks:upgrade-test-2', 'Fresh', 'fresh', 1000,
             'sq', true, NULL, 5);
     INSERT INTO star_track_tiers (sc_track_id, tier, tiered_at, upgrade_from, upgrade_tried_at)
     VALUES ('upgrade-test-1', 1, now() - interval '1 day', 'sq', now() - interval '7 hours'),
            ('upgrade-test-2', 1, now(), 'sq', NULL);";

const LIKED_AND_REST: &str =
    "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms,
                         storage_quality, hq_upgrade_pending, index_priority)
     VALUES ('upgrade-test-3', 'soundcloud:tracks:upgrade-test-3', 'Liked', 'liked', 1000,
             'sq', true, 1),
            ('upgrade-test-4', 'soundcloud:tracks:upgrade-test-4', 'Other', 'other', 1000,
             'sq', true, 5);";

#[tokio::test]
#[ignore = "requires a database with the latest API schema"]
async fn a_star_like_never_tried_goes_before_one_that_failed_earlier()
-> Result<(), Box<dyn std::error::Error>> {
    let mut client = connect().await?;
    let transaction = client.transaction().await?;
    only_these_tracks_wait(&transaction, STAR_LIKES).await?;

    assert_eq!(
        pick_star_liked(&transaction, 1).await?,
        ["soundcloud:tracks:upgrade-test-2"]
    );
    let waited: i64 = transaction
        .query_one(NEXT_STAR_LIKED, &[&COOLDOWN, &"sq", &STAR_SCAN_WINDOW])
        .await?
        .get(0);
    assert!((3_000..4_000).contains(&waited), "waited {waited}s");
    assert_eq!(
        pick_star_liked(&transaction, 10).await?,
        ["soundcloud:tracks:upgrade-test-1"]
    );

    transaction.rollback().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a database with the latest API schema"]
async fn liked_tracks_go_before_the_rest_and_the_rest_keeps_the_old_order()
-> Result<(), Box<dyn std::error::Error>> {
    let mut client = connect().await?;
    let transaction = client.transaction().await?;
    only_these_tracks_wait(&transaction, LIKED_AND_REST).await?;

    assert_eq!(
        pick(&transaction, PICK_LIKED, 10).await?,
        ["soundcloud:tracks:upgrade-test-3"]
    );
    assert_eq!(
        pick(&transaction, PICK_REST, 10).await?,
        ["soundcloud:tracks:upgrade-test-4"]
    );
    let next = transaction
        .query_opt(NEXT_REST, &[&COOLDOWN, &"sq"])
        .await?;
    assert!(next.is_none());

    transaction.rollback().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a database with the latest API schema"]
async fn without_star_track_tiers_only_the_star_pass_fails_and_the_old_order_still_picks()
-> Result<(), Box<dyn std::error::Error>> {
    let mut client = connect().await?;
    let mut transaction = client.transaction().await?;
    only_these_tracks_wait(&transaction, LIKED_AND_REST).await?;

    assert!(pick_star_liked(&transaction, 10).await?.is_empty());
    let missing = transaction.transaction().await?;
    missing.batch_execute("DROP TABLE star_track_tiers").await?;
    let failed = pick_star_liked(&missing, 10)
        .await
        .expect_err("the star pass reads a table that is gone");
    assert_eq!(failed.code(), Some(&SqlState::UNDEFINED_TABLE));
    missing.rollback().await?;
    assert_eq!(
        pick(&transaction, PICK_REST, 10).await?,
        [
            "soundcloud:tracks:upgrade-test-3",
            "soundcloud:tracks:upgrade-test-4"
        ]
    );

    transaction.rollback().await?;
    Ok(())
}

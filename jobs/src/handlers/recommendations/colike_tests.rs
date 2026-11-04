use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;

use super::super::tests::{insert_artist_track, install_schema};
use super::*;

const SINGLE_STATEMENT_REBUILD: &str = include_str!("colike_single_statement.sql");

type Edge = (Uuid, Uuid);
type Edges = BTreeMap<Edge, (i32, u32)>;
type Versions = BTreeMap<Edge, (String, String)>;

fn artist(number: u128) -> Uuid {
    Uuid::from_u128(number)
}

fn edge(a: u128, b: u128) -> Edge {
    (artist(a), artist(b))
}

async fn seed_artists(pool: &PgPool, count: u128) -> anyhow::Result<()> {
    install_schema(pool).await?;
    for number in 1..=count {
        sqlx::query("INSERT INTO artists (id) VALUES ($1)")
            .bind(artist(number))
            .execute(pool)
            .await?;
        insert_artist_track(
            pool,
            artist(number),
            Uuid::from_u128(1000 + number),
            &format!("track-{number}"),
            "ok",
            5,
        )
        .await?;
    }
    Ok(())
}

async fn like(pool: &PgPool, user: &str, artists: &[u128]) -> anyhow::Result<()> {
    for number in artists {
        sqlx::query("INSERT INTO user_likes_tracks (user_id, sc_track_id) VALUES ($1, $2)")
            .bind(user)
            .bind(format!("track-{number}"))
            .execute(pool)
            .await?;
    }
    Ok(())
}

async fn unlike(pool: &PgPool, user: &str, number: u128) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM user_likes_tracks WHERE user_id = $1 AND sc_track_id = $2")
        .bind(user)
        .bind(format!("track-{number}"))
        .execute(pool)
        .await?;
    Ok(())
}

async fn seed_previous_likes(pool: &PgPool) -> anyhow::Result<()> {
    seed_artists(pool, 9).await?;
    for user in ["1", "2"] {
        like(pool, user, &[1, 2, 3]).await?;
    }
    for user in ["3", "4"] {
        like(pool, user, &[4, 5]).await?;
    }
    for user in ["5", "6"] {
        like(pool, user, &[5, 6]).await?;
    }
    for user in ["12", "13"] {
        like(pool, user, &[2, 4]).await?;
    }
    Ok(())
}

async fn change_likes(pool: &PgPool) -> anyhow::Result<()> {
    unlike(pool, "1", 1).await?;
    unlike(pool, "2", 1).await?;
    unlike(pool, "12", 4).await?;
    like(pool, "7", &[4, 5]).await?;
    for user in ["8", "9"] {
        like(pool, user, &[6, 7]).await?;
    }
    for user in ["10", "11"] {
        like(pool, user, &[8, 9]).await?;
    }
    Ok(())
}

async fn seed_listening_windows(pool: &PgPool) -> anyhow::Result<()> {
    seed_artists(pool, 40).await?;
    sqlx::query(
        "INSERT INTO user_likes_tracks (user_id, sc_track_id)
         SELECT listener::text, 'track-' || (listener % 37 + 1 + step)
         FROM generate_series(1, 80) AS listener, generate_series(0, 3) AS step",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn shift_listening_windows(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO user_likes_tracks (user_id, sc_track_id)
         SELECT listener::text, 'track-' || ((listener + pair.shift) % 40 + 1)
         FROM generate_series(81, 160) AS listener, (VALUES (0), (13)) AS pair(shift)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "DELETE FROM user_likes_tracks
         WHERE sc_track_id = 'track-1'
            OR (user_id::int % 3 = 0 AND sc_track_id = 'track-' || (user_id::int % 37 + 1))",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn edges(pool: &PgPool) -> anyhow::Result<Edges> {
    let rows: Vec<(Uuid, Uuid, i32, f32)> =
        sqlx::query_as("SELECT a_id, b_id, co, w FROM artist_colike")
            .fetch_all(pool)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(a, b, co, w)| ((a, b), (co, w.to_bits())))
        .collect())
}

async fn row_versions(pool: &PgPool) -> anyhow::Result<Versions> {
    let rows: Vec<(Uuid, Uuid, String, String)> =
        sqlx::query_as("SELECT a_id, b_id, xmin::text, ctid::text FROM artist_colike")
            .fetch_all(pool)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(a, b, xmin, ctid)| ((a, b), (xmin, ctid)))
        .collect())
}

fn a_ids(edges: &Edges) -> BTreeSet<Uuid> {
    edges.keys().map(|(a, _)| *a).collect()
}

async fn keep_as_previous_run(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query("CREATE TABLE previous_run AS TABLE artist_colike")
        .execute(pool)
        .await?;
    Ok(())
}

async fn restore_previous_run(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "DELETE FROM artist_colike;
         INSERT INTO artist_colike SELECT * FROM previous_run;",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn rebuild_in_one_statement(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(SINGLE_STATEMENT_REBUILD)
        .bind(SHRINKAGE)
        .bind(TOP_K_PER_ARTIST)
        .execute(pool)
        .await?;
    Ok(())
}

async fn rebuild_in_batches_of(pool: &PgPool, batch_edges: i64) -> anyhow::Result<Rebuild> {
    let mut connection = pool.acquire().await?;
    connection.close_on_drop();
    let pacing = Pacing {
        batch_edges,
        pause: Duration::ZERO,
    };
    Ok(rebuild_on(&mut connection, pacing).await?)
}

#[sqlx::test(migrations = false)]
async fn batched_rebuild_ends_where_the_single_statement_ends(pool: PgPool) -> anyhow::Result<()> {
    seed_previous_likes(&pool).await?;
    rebuild_in_one_statement(&pool).await?;
    let previous = edges(&pool).await?;
    change_likes(&pool).await?;
    keep_as_previous_run(&pool).await?;
    rebuild_in_one_statement(&pool).await?;
    let expected = edges(&pool).await?;

    assert_eq!(
        previous.keys().copied().collect::<Vec<_>>(),
        vec![
            edge(1, 2),
            edge(1, 3),
            edge(2, 3),
            edge(2, 4),
            edge(4, 5),
            edge(5, 6)
        ]
    );
    assert_eq!(
        expected.keys().copied().collect::<Vec<_>>(),
        vec![edge(2, 3), edge(4, 5), edge(5, 6), edge(6, 7), edge(8, 9)]
    );
    assert_eq!(previous.get(&edge(2, 3)), expected.get(&edge(2, 3)));
    assert_ne!(previous.get(&edge(4, 5)), expected.get(&edge(4, 5)));
    assert_ne!(previous.get(&edge(5, 6)), expected.get(&edge(5, 6)));
    assert!(a_ids(&previous).contains(&artist(1)) && !a_ids(&expected).contains(&artist(1)));
    assert!(!a_ids(&previous).contains(&artist(8)) && a_ids(&expected).contains(&artist(8)));

    for batch_edges in [1, 2, 3, 500] {
        restore_previous_run(&pool).await?;
        let unchanged = row_versions(&pool).await?.get(&edge(2, 3)).cloned();

        rebuild_in_batches_of(&pool, batch_edges).await?;

        assert_eq!(
            edges(&pool).await?,
            expected,
            "batches of {batch_edges} edges"
        );
        assert_eq!(
            row_versions(&pool).await?.get(&edge(2, 3)).cloned(),
            unchanged,
            "an unchanged edge is not rewritten in batches of {batch_edges}"
        );
    }
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_rebuild_bigger_than_one_batch_commits_batch_by_batch(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_listening_windows(&pool).await?;
    rebuild_in_one_statement(&pool).await?;
    shift_listening_windows(&pool).await?;
    keep_as_previous_run(&pool).await?;
    rebuild_in_one_statement(&pool).await?;
    let expected = edges(&pool).await?;
    restore_previous_run(&pool).await?;
    let previous = row_versions(&pool).await?;

    let rebuild = rebuild_in_batches_of(&pool, 4).await?;

    assert_eq!(edges(&pool).await?, expected);
    let transactions: BTreeSet<String> = row_versions(&pool)
        .await?
        .into_iter()
        .filter(|(key, version)| previous.get(key) != Some(version))
        .map(|(_, (xmin, _))| xmin)
        .collect();
    assert!(
        rebuild.fresh > 12 && rebuild.batches > 3,
        "the fixture must span many batches: {rebuild:?}"
    );
    assert!(
        transactions.len() > 1,
        "the written edges must come from more than one transaction: {transactions:?}"
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_second_rebuild_without_new_likes_writes_nothing(pool: PgPool) -> anyhow::Result<()> {
    seed_previous_likes(&pool).await?;
    rebuild_in_one_statement(&pool).await?;
    change_likes(&pool).await?;

    let first = rebuild_in_batches_of(&pool, 2).await?;
    let written = row_versions(&pool).await?;
    let second = rebuild_in_batches_of(&pool, 2).await?;

    assert!(first.upserted > 0 && first.removed > 0, "{first:?}");
    assert!(second.batches > 1, "{second:?}");
    assert_eq!((second.upserted, second.removed), (0, 0));
    assert_eq!(row_versions(&pool).await?, written);
    Ok(())
}

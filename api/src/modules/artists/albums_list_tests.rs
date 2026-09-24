use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

const ALBUMS_LIST: &str = include_str!("../../../queries/artists/handlers/albums_list.sql");

fn plan_contains(plan: &Value, node_type: &str, field: &str, value: &str) -> bool {
    match plan {
        Value::Object(node) => {
            let is_node = node.get("Node Type").and_then(Value::as_str) == Some(node_type)
                && node.get(field).and_then(Value::as_str) == Some(value);
            is_node
                || node
                    .values()
                    .any(|child| plan_contains(child, node_type, field, value))
        }
        Value::Array(items) => items
            .iter()
            .any(|item| plan_contains(item, node_type, field, value)),
        _ => false,
    }
}

async fn insert_artist(pool: &PgPool, id: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source)
         VALUES ($1, $1::text, $1::text, 'test')",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_album(
    pool: &PgPool,
    id: Uuid,
    title: &str,
    release_year: Option<i16>,
    primary_artist_id: Uuid,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO albums (id, title, normalized_title, source, release_year, primary_artist_id)
         VALUES ($1, $2, $2, 'test', $3, $4)",
    )
    .bind(id)
    .bind(title)
    .bind(release_year)
    .bind(primary_artist_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn credit(pool: &PgPool, album_id: Uuid, artist_id: Uuid, role: &str) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO album_artists (album_id, artist_id, role) VALUES ($1, $2, $3)")
        .bind(album_id)
        .bind(artist_id)
        .bind(role)
        .execute(pool)
        .await?;
    Ok(())
}

async fn wanted_on_album(pool: &PgPool, album_id: Uuid, artist_id: Uuid) -> anyhow::Result<()> {
    let wanted_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO wanted_tracks (id, title, normalized_title, source, primary_artist_id)
         VALUES ($1, 'wanted', 'wanted', 'test', $2)",
    )
    .bind(wanted_id)
    .bind(artist_id)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO wanted_track_albums (wanted_track_id, album_id) VALUES ($1, $2)")
        .bind(wanted_id)
        .bind(album_id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn listing(pool: &PgPool, artist_id: Uuid) -> anyhow::Result<Vec<(Uuid, String)>> {
    let rows = sqlx::query_as::<_, (Uuid, String)>(&format!(
        "SELECT id, \"role!\" FROM ({ALBUMS_LIST}) AS listing"
    ))
    .bind(artist_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[sqlx::test(migrations = "./migrations")]
async fn the_listing_joins_primary_credited_and_wanted_albums_in_release_order(
    pool: PgPool,
) -> anyhow::Result<()> {
    let subject = Uuid::from_u128(1);
    let other = Uuid::from_u128(2);
    insert_artist(&pool, subject).await?;
    insert_artist(&pool, other).await?;

    let primary = Uuid::from_u128(10);
    let credited = Uuid::from_u128(11);
    let primary_and_credited = Uuid::from_u128(12);
    let wanted = Uuid::from_u128(13);
    let unrelated = Uuid::from_u128(14);
    insert_album(&pool, primary, "Primary", Some(2021), subject).await?;
    insert_album(&pool, credited, "Alpha", Some(2020), other).await?;
    insert_album(&pool, primary_and_credited, "Beta", Some(2020), subject).await?;
    insert_album(&pool, wanted, "Wanted", None, other).await?;
    insert_album(&pool, unrelated, "Unrelated", Some(2022), other).await?;
    credit(&pool, credited, subject, "producer").await?;
    credit(&pool, credited, subject, "remixer").await?;
    credit(&pool, primary_and_credited, subject, "featured").await?;
    credit(&pool, unrelated, other, "producer").await?;
    wanted_on_album(&pool, wanted, subject).await?;
    wanted_on_album(&pool, primary, subject).await?;

    let rows = listing(&pool, subject).await?;

    assert_eq!(
        rows,
        vec![
            (primary, "primary".to_owned()),
            (credited, "producer".to_owned()),
            (primary_and_credited, "primary".to_owned()),
            (wanted, "featured".to_owned()),
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_second_credit_on_the_same_album_does_not_duplicate_it(
    pool: PgPool,
) -> anyhow::Result<()> {
    let subject = Uuid::from_u128(1);
    let other = Uuid::from_u128(2);
    insert_artist(&pool, subject).await?;
    insert_artist(&pool, other).await?;
    let album = Uuid::from_u128(10);
    insert_album(&pool, album, "Split", Some(2020), other).await?;
    credit(&pool, album, subject, "remixer").await?;
    credit(&pool, album, subject, "primary").await?;
    credit(&pool, album, subject, "featured").await?;

    assert_eq!(
        listing(&pool, subject).await?,
        vec![(album, "primary".to_owned())]
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_listing_reaches_albums_through_indexes_instead_of_scanning_the_catalog(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO artists (id, name, normalized_name, source)
         SELECT ('00000000-0000-4000-8001-' || lpad(to_hex(n), 12, '0'))::uuid, 'a' || n, 'a' || n, 'test'
         FROM generate_series(1, 2000) AS n;
         INSERT INTO albums (id, title, normalized_title, source, primary_artist_id)
         SELECT gen_random_uuid(), 'al' || n, 'al' || n, 'test',
                ('00000000-0000-4000-8001-' || lpad(to_hex(1 + (n * 7919) % 2000), 12, '0'))::uuid
         FROM generate_series(1, 50000) AS n;
         INSERT INTO album_artists (album_id, artist_id, role)
         SELECT id,
                ('00000000-0000-4000-8001-' || lpad(to_hex(1 + row_number() OVER () % 2000), 12, '0'))::uuid,
                'featured'
         FROM albums;
         INSERT INTO wanted_tracks (id, title, normalized_title, source, primary_artist_id)
         SELECT gen_random_uuid(), 'w' || n, 'w' || n, 'test',
                ('00000000-0000-4000-8001-' || lpad(to_hex(1 + n % 2000), 12, '0'))::uuid
         FROM generate_series(1, 20000) AS n;
         INSERT INTO wanted_track_albums (wanted_track_id, album_id)
         SELECT wanted.id, album.id
         FROM (SELECT id, row_number() OVER () AS rn FROM wanted_tracks) AS wanted
         JOIN (SELECT id, row_number() OVER () AS rn FROM albums) AS album ON album.rn = wanted.rn;
         ANALYZE artists, albums, album_artists, wanted_tracks, wanted_track_albums;",
    )
    .execute(&pool)
    .await?;

    let plan: Value = sqlx::query_scalar(&format!("EXPLAIN (FORMAT JSON) {ALBUMS_LIST}"))
        .bind(Uuid::parse_str("00000000-0000-4000-8001-000000000001")?)
        .fetch_one(&pool)
        .await?;

    assert!(
        !plan_contains(&plan, "Seq Scan", "Relation Name", "albums"),
        "one artist's albums must not be found by scanning every album: {plan}"
    );
    assert!(
        plan.to_string().contains("albums_primary_artist_idx"),
        "the primary-artist index must drive the lookup: {plan}"
    );
    Ok(())
}

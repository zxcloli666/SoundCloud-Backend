use sqlx::PgPool;

use super::*;
use crate::modules::recommendations::smart_wave::blocked::blocked_uploads;

const ARTIST: &str = "7d2b1f4c-3b1a-4f0e-9a51-0c5f6f2e1a10";

fn input(name: &str, accounts: &[&str]) -> BlockInput {
    BlockInput {
        name: name.into(),
        avatar_url: Some("https://i1.sndcdn.com/a.jpg".into()),
        sc_user_ids: accounts.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn targets_are_normalized_per_kind() {
    assert_eq!(
        normalize_target(BlockKind::User, "soundcloud:users:17").unwrap(),
        "17"
    );
    assert_eq!(normalize_target(BlockKind::User, "17").unwrap(), "17");
    assert!(normalize_target(BlockKind::User, "abc").is_err());
    assert_eq!(
        normalize_target(BlockKind::Artist, &ARTIST.to_uppercase()).unwrap(),
        ARTIST
    );
    assert!(normalize_target(BlockKind::Artist, "17").is_err());
    assert!(BlockKind::parse("label").is_err());
}

#[test]
fn linked_accounts_keep_the_user_first_and_drop_junk() {
    let raw = vec![
        "soundcloud:users:9".to_string(),
        "17".to_string(),
        "x".to_string(),
        "9".to_string(),
    ];
    assert_eq!(
        linked_accounts(BlockKind::User, "17", &raw),
        vec!["17", "9"]
    );
    assert_eq!(
        linked_accounts(BlockKind::Artist, ARTIST, &raw),
        vec!["9", "17"]
    );
    let many: Vec<String> = (1..100).map(|i| i.to_string()).collect();
    assert_eq!(
        linked_accounts(BlockKind::Artist, ARTIST, &many).len(),
        MAX_LINKED_ACCOUNTS
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn block_list_and_unblock_round_trip(pool: PgPool) -> anyhow::Result<()> {
    block(
        &pool,
        "5",
        BlockKind::User,
        "soundcloud:users:17",
        &input(" Uploader ", &[]),
    )
    .await?;
    let artist = block(
        &pool,
        "soundcloud:users:5",
        BlockKind::Artist,
        ARTIST,
        &input("Artist", &["44"]),
    )
    .await?;
    assert_eq!(artist.sc_user_ids, vec!["44"]);

    let listed = list(&pool, "5").await?;
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].kind, BlockKind::Artist);
    assert_eq!(listed[1].id, "17");
    assert_eq!(listed[1].name, "Uploader");
    assert_eq!(listed[1].sc_user_ids, vec!["17"]);

    block(&pool, "5", BlockKind::User, "17", &input("Renamed", &[])).await?;
    assert_eq!(list(&pool, "5").await?.len(), 2);

    unblock(&pool, "soundcloud:users:5", BlockKind::User, "17").await?;
    let left = list(&pool, "5").await?;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, ARTIST);
    assert!(list(&pool, "6").await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn blocklist_is_capped(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO user_blocked_artists (sc_user_id, kind, target_id, name)
         SELECT '5', 'user', g::text, 'n' FROM generate_series(1, $1) g",
    )
    .bind(MAX_BLOCKED as i32)
    .execute(&pool)
    .await?;
    assert!(
        block(&pool, "5", BlockKind::User, "999999", &input("x", &[]))
            .await
            .is_err()
    );
    block(&pool, "5", BlockKind::User, "1", &input("Again", &[])).await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn wave_drops_blocked_uploads_and_artists(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source) VALUES ($1::uuid, 'A', 'a', 'test'),
         ('11111111-1111-4111-8111-111111111111'::uuid, 'B', 'b', 'test')",
    )
    .bind(ARTIST)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO artist_sc_accounts (artist_id, sc_user_id, role, source)
         VALUES ('11111111-1111-4111-8111-111111111111'::uuid, '17', 'main', 'test')",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, uploader_sc_user_id)
         VALUES ('1', 'soundcloud:tracks:1', 'a', 'a', 1000, '17'),
                ('2', 'soundcloud:tracks:2', 'b', 'b', 1000, '44'),
                ('3', 'soundcloud:tracks:3', 'c', 'c', 1000, '99')",
    )
    .execute(&pool)
    .await?;
    block(&pool, "5", BlockKind::User, "17", &input("U", &[])).await?;
    block(&pool, "5", BlockKind::Artist, ARTIST, &input("A", &["44"])).await?;

    let variants = vec!["5".to_string()];
    let mut dropped: Vec<u64> = blocked_uploads(&pool, &variants, &[1, 2, 3])
        .await
        .into_iter()
        .collect();
    dropped.sort_unstable();
    assert_eq!(dropped, vec![1, 2]);

    let mut artists: Vec<uuid::Uuid> = sqlx::query_file_scalar!(
        "queries/recommendations/smart_wave/graph/load_disliked_artists.sql",
        &variants,
        3i64
    )
    .fetch_all(&pool)
    .await?;
    artists.sort();
    let mut expected = vec![
        uuid::Uuid::parse_str(ARTIST)?,
        uuid::Uuid::parse_str("11111111-1111-4111-8111-111111111111")?,
    ];
    expected.sort();
    assert_eq!(artists, expected);
    Ok(())
}

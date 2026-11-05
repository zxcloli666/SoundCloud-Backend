use super::{UserRepository, project_to_sc_shape};
use sqlx::PgPool;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn numeric_user_id_reads_and_touches_the_canonical_local_profile(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO users (sc_user_id, urn, username, username_normalized)
        VALUES ('17', 'soundcloud:users:17', 'Local user', 'local user')",
    )
    .execute(&pool)
    .await?;
    let repo = UserRepository::new(pool.clone());
    let numeric = repo.find_by_urn("17").await?.expect("numeric id");
    let canonical = repo
        .find_by_urn("soundcloud:users:17")
        .await?
        .expect("canonical urn");
    assert_eq!(numeric.urn, canonical.urn);
    repo.touch_last_read("17").await?;
    let read: Vec<String> = sqlx::query_scalar("SELECT entity_urn FROM entity_reads")
        .fetch_all(&pool)
        .await?;
    assert_eq!(read, ["soundcloud:users:17"]);
    assert!(
        repo.find_by_urn("soundcloud:users:17")
            .await?
            .expect("touched user")
            .last_read_at
            .is_none(),
        "a read no longer rewrites the user row"
    );
    assert!(repo.find_by_urn("soundcloud:tracks:17").await?.is_none());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_stored_banner_is_served_with_the_user(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO users (sc_user_id, urn, username, username_normalized, banner_url)
        VALUES ('17', 'soundcloud:users:17', 'Owner', 'owner', 'https://i1.sndcdn.com/visuals-000000000017-LpaCgv-original.jpg'),
               ('18', 'soundcloud:users:18', 'Plain', 'plain', NULL)",
    )
    .execute(&pool)
    .await?;
    let repo = UserRepository::new(pool.clone());
    let bannered = project_to_sc_shape(&repo.find_by_urn("17").await?.expect("user 17"));
    let plain = project_to_sc_shape(&repo.find_by_urn("18").await?.expect("user 18"));
    assert_eq!(
        bannered["banner_url"],
        "https://i1.sndcdn.com/visuals-000000000017-LpaCgv-original.jpg"
    );
    assert!(plain.get("banner_url").is_none());
    Ok(())
}

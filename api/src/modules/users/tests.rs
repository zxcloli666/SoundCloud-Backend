use super::UserRepository;
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn numeric_user_id_reads_and_touches_the_canonical_local_profile(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO users (sc_user_id, urn, username, username_normalized)
        VALUES ('17', 'soundcloud:users:17', 'Local user', 'local user')",
    )
    .execute(&pool)
    .await?;
    let repo = UserRepository::new(pool);
    let numeric = repo.find_by_urn("17").await?.expect("numeric id");
    let canonical = repo
        .find_by_urn("soundcloud:users:17")
        .await?
        .expect("canonical urn");
    assert_eq!(numeric.urn, canonical.urn);
    assert!(numeric.last_read_at.is_none());
    repo.touch_last_read("17").await?;
    assert!(
        repo.find_by_urn("soundcloud:users:17")
            .await?
            .expect("touched user")
            .last_read_at
            .is_some()
    );
    assert!(repo.find_by_urn("soundcloud:tracks:17").await?.is_none());
    Ok(())
}

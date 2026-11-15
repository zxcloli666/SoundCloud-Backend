use sqlx::PgPool;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_projected_track_carries_its_moment_only_when_one_is_known(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, sharing, duration_ms)
         VALUES ('71', 'soundcloud:tracks:71', 'Hook', 'hook', 'public', 200000),
                ('72', 'soundcloud:tracks:72', 'Plain', 'plain', 'public', 200000);
         INSERT INTO track_highlights (track_id, start_ms, length_ms, score, version)
         SELECT id, 52500, 30000, 0.8, 1 FROM tracks WHERE sc_track_id = '71'",
    )
    .execute(&pool)
    .await?;

    let projected =
        crate::modules::tracks::project_many_public(&pool, &["71".into(), "72".into()]).await?;
    let [Some(hooked), Some(plain)] = projected.as_slice() else {
        anyhow::bail!("both tracks project");
    };

    assert_eq!(
        hooked["highlight"],
        serde_json::json!({"startMs": 52500, "lengthMs": 30000})
    );
    assert!(plain.get("highlight").is_none());
    Ok(())
}

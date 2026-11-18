use sqlx::PgPool;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_projected_track_carries_its_video_every_verified_video_by_kind(
    pool: PgPool,
) -> anyhow::Result<()> {
    super::set_base_url("https://storage.test/");
    sqlx::raw_sql(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, sharing, duration_ms)
         VALUES ('81', 'soundcloud:tracks:81', 'Clip', 'clip', 'public', 200000),
                ('82', 'soundcloud:tracks:82', 'Plain', 'plain', 'public', 200000);
         INSERT INTO track_videos (sc_track_id, kind, source, s3_key, offset_ms, loop, duration_ms)
         VALUES ('81', 'generated', 'generated', 'video/81-1.mp4', 0, true, 30000),
                ('81', 'clip', 'youtube', 'video/81-2.mp4', 1500, false, 200000),
                ('81', 'loop', 'youtube', 'video/81-3.mp4', 0, true, 12000)",
    )
    .execute(&pool)
    .await?;

    let projected =
        crate::modules::tracks::project_many_public(&pool, &["81".into(), "82".into()]).await?;
    let [Some(clip), Some(plain)] = projected.as_slice() else {
        anyhow::bail!("both tracks project");
    };

    assert_eq!(
        clip["videos"],
        serde_json::json!([
            {"kind": "clip", "url": "https://storage.test/video/81-2.mp4", "offsetMs": 1500, "loop": false},
            {"kind": "loop", "url": "https://storage.test/video/81-3.mp4", "offsetMs": 0, "loop": true},
            {"kind": "generated", "url": "https://storage.test/video/81-1.mp4", "offsetMs": 0, "loop": true},
        ])
    );
    assert!(clip.get("video").is_none());
    assert!(plain.get("videos").is_none());
    Ok(())
}

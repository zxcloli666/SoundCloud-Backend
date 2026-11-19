use backend_contracts::admin_video::VideoSource;
use backend_contracts::pipeline::{VideoCandidateKind, VideoKind};
use backend_contracts::reasons::WorkerReason;
use sqlx::PgPool;

use super::super::queue;
use super::super::sources::Lead;
use super::{
    Memory, Rig, account, claim, done, finish, loop_message, part_message, rig, star_track,
};

async fn with_video(pool: &PgPool, blobs: &Memory, id: &str, kind: &str) -> anyhow::Result<()> {
    let key = format!("video/{id}-1.mp4");
    sqlx::query(
        "INSERT INTO track_videos (sc_track_id, kind, source, source_ref, s3_key, loop, duration_ms)
         VALUES ($1, $2, 'youtube', 'aaa', $3, $2 = 'loop', 90000)",
    )
    .bind(id)
    .bind(kind)
    .bind(&key)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO track_video_dispatch (sc_track_id, stage, attempts, next_at)
         VALUES ($1, 'found', 1, now() - interval '1 day')",
    )
    .bind(id)
    .execute(pool)
    .await?;
    blobs.put_at(&key, b"old video", std::time::SystemTime::now());
    Ok(())
}

fn youtube(source_ref: &str) -> Lead {
    Lead {
        source: VideoSource::Youtube,
        kind: VideoCandidateKind::Video,
        url: format!("https://www.youtube.com/watch?v={source_ref}"),
        source_ref: Some(source_ref.to_owned()),
    }
}

async fn research(pool: &PgPool, video: &Rig, id: &str) -> anyhow::Result<queue::Claimed> {
    let claimed = claim(pool, 1).await?.remove(0);
    assert_eq!(claimed.sc_track_id, id);
    let leased = video.lease_youtube().await?.unwrap();
    let kept = queue::kept(pool, id)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let leads = vec![youtube("aaa"), youtube("bbb")];
    let staged = video
        .bring_all(&claimed, kept, leads, &leased)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let marked = queue::stage(pool, &claimed, &staged)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    assert!(marked);
    Ok(claimed)
}

async fn videos(pool: &PgPool, id: &str) -> anyhow::Result<Vec<(String, Option<String>, String)>> {
    Ok(sqlx::query_as(
        "SELECT kind, source_ref, s3_key FROM track_videos WHERE sc_track_id = $1 ORDER BY kind",
    )
    .bind(id)
    .fetch_all(pool)
    .await?)
}

async fn wait_days(pool: &PgPool, id: &str) -> anyhow::Result<(String, i64)> {
    Ok(sqlx::query_as(
        "SELECT stage, (extract(epoch FROM next_at - now()) / 86400)::bigint
         FROM track_video_dispatch WHERE sc_track_id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?)
}

fn kept_wins(id: &str) -> backend_contracts::pipeline::VideoFindResult {
    let mut result = done(id, 2, None);
    result.kind = Some(VideoKind::Clip);
    result.candidate = Some(0);
    result.looped = Some(false);
    result
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_stored_video_is_searched_again_in_a_month_when_the_track_is_fresh_and_in_a_quarter_when_old(
    pool: PgPool,
) -> anyhow::Result<()> {
    star_track(&pool, "11", 3, 1).await?;
    star_track(&pool, "12", 2, 1).await?;
    star_track(&pool, "13", 1, 1).await?;
    sqlx::query(
        "UPDATE tracks SET release_date = CASE sc_track_id
             WHEN '11' THEN (now() - interval '30 days')::date
             WHEN '12' THEN (now() - interval '400 days')::date
         END,
         sc_created_at = CASE WHEN sc_track_id = '13' THEN now() - interval '10 days' END",
    )
    .execute(&pool)
    .await?;
    let (video, _, blobs, _) = rig(&pool);
    let file: &[u8] = b"the encoded loop";

    for id in ["11", "12", "13"] {
        super::staged_claim(&pool, &blobs, id, 2).await?;
        finish(&video, &loop_message(id, 1, file, &[file], 0)).await?;
    }

    assert_eq!(wait_days(&pool, "11").await?, ("found".to_owned(), 30));
    assert_eq!(wait_days(&pool, "12").await?, ("found".to_owned(), 90));
    assert_eq!(wait_days(&pool, "13").await?, ("found".to_owned(), 30));
    assert!(claim(&pool, 10).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_due_research_ranks_below_a_new_track_as_popular_and_above_one_ten_times_less_popular(
    pool: PgPool,
) -> anyhow::Result<()> {
    let blobs = Memory::default();
    star_track(&pool, "11", 1_000, 1).await?;
    star_track(&pool, "12", 1_000, 1).await?;
    star_track(&pool, "13", 100, 1).await?;
    with_video(&pool, &blobs, "11", "clip").await?;

    let order = [
        claim(&pool, 1).await?.remove(0),
        claim(&pool, 1).await?.remove(0),
        claim(&pool, 1).await?.remove(0),
    ];

    assert_eq!(
        order
            .iter()
            .map(|claimed| (claimed.sc_track_id.as_str(), claimed.attempt))
            .collect::<Vec<_>>(),
        [("12", 1), ("11", 2), ("13", 1)]
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_research_passes_the_stored_video_skips_its_source_and_changes_nothing_when_it_wins(
    pool: PgPool,
) -> anyhow::Result<()> {
    account(&pool, "yt1", "youtube").await?;
    star_track(&pool, "11", 1, 1).await?;
    let (video, _, blobs, _) = rig(&pool);
    with_video(&pool, &blobs, "11", "clip").await?;
    let claimed = research(&pool, &video, "11").await?;
    let staged = queue::staged(&pool, "11", claimed.attempt)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?
        .unwrap();
    let sent = video
        .candidate_list(&staged)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    assert_eq!(
        sent.iter()
            .map(|candidate| (candidate.index, candidate.url.as_str(), candidate.kept))
            .collect::<Vec<_>>(),
        [
            (0, "https://storage.test/video/11-1.mp4", Some(true)),
            (1, "https://storage.test/video-tmp/11-2-1.mp4", None),
        ]
    );

    finish(&video, &kept_wins("11")).await?;

    assert_eq!(
        videos(&pool, "11").await?,
        [(
            "clip".to_owned(),
            Some("aaa".to_owned()),
            "video/11-1.mp4".to_owned()
        )]
    );
    assert_eq!(blobs.keys(), ["video/11-1.mp4"]);
    assert_eq!(
        blobs.get("video/11-1.mp4").as_deref(),
        Some(&b"old video"[..])
    );
    assert_eq!(wait_days(&pool, "11").await?, ("found".to_owned(), 90));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_better_clip_replaces_the_stored_one_and_the_old_file_goes(
    pool: PgPool,
) -> anyhow::Result<()> {
    account(&pool, "yt1", "youtube").await?;
    star_track(&pool, "11", 1, 1).await?;
    let (video, _, blobs, _) = rig(&pool);
    with_video(&pool, &blobs, "11", "clip").await?;
    research(&pool, &video, "11").await?;
    let file: &[u8] = b"the new clip";

    finish(&video, &part_message("11", 2, file, &[file], 0)).await?;

    assert_eq!(
        videos(&pool, "11").await?,
        [(
            "clip".to_owned(),
            Some("bbb".to_owned()),
            "video/11-2.mp4".to_owned()
        )]
    );
    assert_eq!(blobs.keys(), ["video/11-2.mp4"]);
    assert_eq!(wait_days(&pool, "11").await?.0, "found");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_clip_found_for_a_track_with_only_a_loop_is_added_next_to_the_loop(
    pool: PgPool,
) -> anyhow::Result<()> {
    account(&pool, "yt1", "youtube").await?;
    star_track(&pool, "11", 1, 1).await?;
    let (video, _, blobs, _) = rig(&pool);
    with_video(&pool, &blobs, "11", "loop").await?;
    research(&pool, &video, "11").await?;
    let file: &[u8] = b"the new clip";

    finish(&video, &part_message("11", 2, file, &[file], 0)).await?;

    assert_eq!(
        videos(&pool, "11").await?,
        [
            (
                "clip".to_owned(),
                Some("bbb".to_owned()),
                "video/11-2.mp4".to_owned()
            ),
            (
                "loop".to_owned(),
                Some("aaa".to_owned()),
                "video/11-1.mp4".to_owned()
            ),
        ]
    );
    assert_eq!(blobs.keys(), ["video/11-1.mp4", "video/11-2.mp4"]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn static_or_nothing_on_a_research_keeps_the_stored_video(
    pool: PgPool,
) -> anyhow::Result<()> {
    account(&pool, "yt1", "youtube").await?;
    star_track(&pool, "11", 200, 1).await?;
    star_track(&pool, "12", 100, 1).await?;
    let (video, _, blobs, _) = rig(&pool);
    with_video(&pool, &blobs, "11", "clip").await?;
    with_video(&pool, &blobs, "12", "loop").await?;
    research(&pool, &video, "11").await?;
    research(&pool, &video, "12").await?;

    finish(&video, &done("11", 2, Some(WorkerReason::StaticVideo))).await?;
    finish(&video, &done("12", 2, Some(WorkerReason::NoVideoFound))).await?;

    assert_eq!(videos(&pool, "11").await?[0].2, "video/11-1.mp4");
    assert_eq!(videos(&pool, "12").await?[0].2, "video/12-1.mp4");
    assert_eq!(blobs.keys(), ["video/11-1.mp4", "video/12-1.mp4"]);
    assert_eq!(wait_days(&pool, "11").await?, ("found".to_owned(), 90));
    assert_eq!(wait_days(&pool, "12").await?, ("found".to_owned(), 90));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_uncertain_new_clip_on_a_research_waits_for_the_vlm_while_the_old_video_stays(
    pool: PgPool,
) -> anyhow::Result<()> {
    account(&pool, "yt1", "youtube").await?;
    star_track(&pool, "11", 1, 1).await?;
    let (video, _, blobs, _) = rig(&pool);
    with_video(&pool, &blobs, "11", "clip").await?;
    research(&pool, &video, "11").await?;
    let mut uncertain = done("11", 2, None);
    uncertain.uncertain = Some(true);

    finish(&video, &uncertain).await?;

    assert_eq!(wait_days(&pool, "11").await?.0, "pending_vlm");
    assert_eq!(videos(&pool, "11").await?[0].2, "video/11-1.mp4");
    assert_eq!(blobs.keys(), ["video-tmp/11-2-1.mp4", "video/11-1.mp4"]);
    Ok(())
}

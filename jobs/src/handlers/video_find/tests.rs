use std::collections::BTreeMap;
use std::time::Duration;

use backend_contracts::admin_video::VideoSource;
use backend_contracts::pipeline::{Producer, VideoFindResult, VideoKind};
use backend_contracts::reasons::{WorkerReason, WorkerStatus};
use sqlx::PgPool;

use super::Pacing;
use super::queue;
use super::result::{self, Verdict};
use crate::handlers::video_accounts::store;

const PAUSE: Duration = Duration::from_secs(3_600);

fn pacing() -> Pacing {
    Pacing {
        account_pause: PAUSE,
        retry_base: Duration::from_secs(900),
        max_attempts: 3,
        none_recheck: Duration::from_secs(30 * 86_400),
        static_recheck: Duration::from_secs(90 * 86_400),
    }
}

async fn star_track(pool: &PgPool, id: &str, plays: i64, tier: i16) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, sharing, duration_ms,
                             storage_state, play_count_sc, metadata_artist)
         VALUES ($1, 'soundcloud:tracks:' || $1, 'Song', 'song', 'public', 200000, 'ok', $2, 'Artist')",
    )
    .bind(id)
    .bind(plays)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO star_track_tiers (sc_track_id, tier) VALUES ($1, $2)")
        .bind(id)
        .bind(tier)
        .execute(pool)
        .await?;
    Ok(())
}

async fn account(pool: &PgPool, name: &str, source: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO video_accounts (source, name, secret, status) VALUES ($1, $2, 'cookie', 'ok')",
    )
    .bind(source)
    .bind(name)
    .execute(pool)
    .await?;
    Ok(())
}

async fn claim(pool: &PgPool, limit: i64) -> anyhow::Result<Vec<queue::Claimed>> {
    queue::claim(pool, limit)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

async fn stage(pool: &PgPool, id: &str) -> anyhow::Result<(String, i32, Option<String>)> {
    Ok(sqlx::query_as(
        "SELECT stage, attempts, last_error FROM track_video_dispatch WHERE sc_track_id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?)
}

fn done(id: &str, attempt: i64, reason: Option<WorkerReason>) -> VideoFindResult {
    let found = reason.is_none();
    VideoFindResult {
        sc_track_id: id.to_owned(),
        attempt,
        status: reason.map_or(WorkerStatus::Ok, WorkerReason::status),
        reason,
        detail: None,
        producer: Producer {
            worker_id: "sc-worker1".to_owned(),
            build: "test".to_owned(),
            models: BTreeMap::new(),
            sync_version: None,
        },
        account: Some("yt1".to_owned()),
        kind: found.then_some(VideoKind::Clip),
        uncertain: found.then_some(false),
        s3_key: found.then(|| format!("video/{id}-{attempt}.mp4")),
        offset_ms: found.then_some(3_839),
        duration_ms: found.then_some(96_000),
        looped: found.then_some(false),
        source: found.then_some(VideoSource::Youtube),
        source_ref: found.then(|| "_VuJA-VQRcY".to_owned()),
        scores: found.then(|| BTreeMap::from([("ber".to_owned(), 0.02)])),
    }
}

async fn record(pool: &PgPool, result: &VideoFindResult) -> anyhow::Result<()> {
    result::record(pool, &pacing(), result)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn only_tier_one_tracks_without_a_video_are_claimed_most_played_first(
    pool: PgPool,
) -> anyhow::Result<()> {
    star_track(&pool, "11", 10, 1).await?;
    star_track(&pool, "12", 5_000, 1).await?;
    star_track(&pool, "13", 99_999, 3).await?;
    star_track(&pool, "14", 700, 1).await?;
    sqlx::query(
        "INSERT INTO track_videos (sc_track_id, kind, source, s3_key, duration_ms)
         VALUES ('14', 'clip', 'youtube', 'video/14-1.mp4', 90000)",
    )
    .execute(&pool)
    .await?;

    let first = claim(&pool, 1).await?;
    let rest = claim(&pool, 10).await?;

    assert_eq!(first[0].sc_track_id, "12");
    assert_eq!(
        rest.iter()
            .map(|claim| claim.sc_track_id.as_str())
            .collect::<Vec<_>>(),
        ["11"]
    );
    assert!(
        first
            .iter()
            .chain(&rest)
            .all(|claim| claim.attempt == 1 && claim.artist == "Artist")
    );
    assert!(
        claim(&pool, 10).await?.is_empty(),
        "claimed tracks are in flight"
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn accounts_rotate_and_a_busy_paused_or_dead_one_is_skipped(
    pool: PgPool,
) -> anyhow::Result<()> {
    account(&pool, "yt1", "youtube").await?;
    account(&pool, "yt2", "youtube").await?;
    account(&pool, "yt3", "youtube").await?;
    account(&pool, "sp1", "spotify").await?;
    star_track(&pool, "11", 1, 1).await?;
    let first = claim(&pool, 1).await?;
    let first_account = store::lease(&pool, "youtube").await?.map(|(name, _)| name);
    assert_eq!(first_account.as_deref(), Some("yt1"));
    queue::mark_account(&pool, &first[0], "yt1")
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    store::pause(&pool, "yt2", PAUSE, "429").await?;
    store::kill(&pool, "yt3", "dead").await?;

    assert_eq!(
        store::lease(&pool, "youtube").await?,
        None,
        "yt1 is busy, yt2 paused, yt3 dead"
    );
    assert_eq!(
        store::lease(&pool, "spotify")
            .await?
            .map(|(name, _)| name)
            .as_deref(),
        Some("sp1")
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_found_clip_is_stored_and_a_stale_answer_is_ignored(pool: PgPool) -> anyhow::Result<()> {
    star_track(&pool, "11", 1, 1).await?;
    let claimed = claim(&pool, 1).await?;
    record(&pool, &done("11", i64::from(claimed[0].attempt) + 1, None)).await?;
    let none: Option<(String,)> =
        sqlx::query_as("SELECT kind FROM track_videos WHERE sc_track_id = '11'")
            .fetch_optional(&pool)
            .await?;
    assert_eq!(none, None);

    record(&pool, &done("11", i64::from(claimed[0].attempt), None)).await?;

    let stored: (String, i32, bool, String) = sqlx::query_as(
        "SELECT kind, offset_ms, loop, s3_key FROM track_videos WHERE sc_track_id = '11'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        stored,
        ("clip".to_owned(), 3_839, false, "video/11-1.mp4".to_owned())
    );
    assert_eq!(stage(&pool, "11").await?.0, "found");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_uncertain_clip_is_held_for_the_vlm_and_not_stored(pool: PgPool) -> anyhow::Result<()> {
    star_track(&pool, "11", 1, 1).await?;
    let claimed = claim(&pool, 1).await?;
    let mut uncertain = done("11", i64::from(claimed[0].attempt), None);
    uncertain.uncertain = Some(true);

    record(&pool, &uncertain).await?;

    let stored: (i64,) = sqlx::query_as("SELECT count(*) FROM track_videos")
        .fetch_one(&pool)
        .await?;
    let held: (Option<String>,) = sqlx::query_as(
        "SELECT payload ->> 's3_key' FROM track_video_dispatch WHERE sc_track_id = '11'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(stored.0, 0);
    assert_eq!(stage(&pool, "11").await?.0, "pending_vlm");
    assert_eq!(held.0.as_deref(), Some("video/11-1.mp4"));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn nothing_found_waits_a_month_and_a_static_video_a_quarter(
    pool: PgPool,
) -> anyhow::Result<()> {
    star_track(&pool, "11", 2, 1).await?;
    star_track(&pool, "12", 1, 1).await?;
    let claimed = claim(&pool, 2).await?;
    record(&pool, &done("11", 1, Some(WorkerReason::NoVideoFound))).await?;
    record(&pool, &done("12", 1, Some(WorkerReason::StaticVideo))).await?;

    let waits: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT sc_track_id, stage, (extract(epoch FROM next_at - now()) / 86400)::bigint
         FROM track_video_dispatch ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(claimed.len(), 2);
    assert_eq!(waits[0], ("11".to_owned(), "none".to_owned(), 30));
    assert_eq!(waits[1], ("12".to_owned(), "static".to_owned(), 90));
    assert!(claim(&pool, 5).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_rate_limit_pauses_the_account_and_does_not_count_the_attempt(
    pool: PgPool,
) -> anyhow::Result<()> {
    account(&pool, "yt1", "youtube").await?;
    star_track(&pool, "11", 1, 1).await?;
    claim(&pool, 1).await?;

    record(&pool, &done("11", 1, Some(WorkerReason::SourceRateLimited))).await?;

    assert_eq!(stage(&pool, "11").await?.0, "retry");
    assert_eq!(stage(&pool, "11").await?.1, 0);
    let paused: (bool,) =
        sqlx::query_as("SELECT paused_until > now() FROM video_accounts WHERE name = 'yt1'")
            .fetch_one(&pool)
            .await?;
    assert!(paused.0);
    assert_eq!(store::lease(&pool, "youtube").await?, None);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_auth_failure_marks_the_account_dead_through_the_account_path(
    pool: PgPool,
) -> anyhow::Result<()> {
    account(&pool, "yt1", "youtube").await?;
    star_track(&pool, "11", 1, 1).await?;
    claim(&pool, 1).await?;

    record(&pool, &done("11", 1, Some(WorkerReason::SourceAuthFailed))).await?;

    let dead: (String, Option<String>) =
        sqlx::query_as("SELECT status, last_error FROM video_accounts WHERE name = 'yt1'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        dead,
        ("dead".to_owned(), Some("source_auth_failed".to_owned()))
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_transient_failure_backs_off_and_gives_up_at_the_attempt_cap(
    pool: PgPool,
) -> anyhow::Result<()> {
    star_track(&pool, "11", 1, 1).await?;
    for attempt in 1..=3 {
        sqlx::query("UPDATE track_video_dispatch SET next_at = now() WHERE sc_track_id = '11'")
            .execute(&pool)
            .await?;
        let claimed = claim(&pool, 1).await?;
        assert_eq!(claimed[0].attempt, attempt);
        record(
            &pool,
            &done("11", i64::from(attempt), Some(WorkerReason::DownloadFailed)),
        )
        .await?;
        let expected = if attempt < 3 { "retry" } else { "none" };
        assert_eq!(stage(&pool, "11").await?.0, expected);
    }
    Ok(())
}

#[test]
fn a_result_with_a_foreign_key_or_a_missing_window_is_refused() {
    let mut outside = done("11", 1, None);
    outside.s3_key = Some("audio/11.opus".to_owned());
    assert!(result::judge(&outside).is_err());
    let mut windowless = done("11", 1, None);
    windowless.duration_ms = None;
    assert!(result::judge(&windowless).is_err());
    assert!(matches!(
        result::judge(&done("11", 1, None)),
        Ok(Verdict::Found(_))
    ));
}

#[test]
fn static_and_empty_results_settle_while_transient_ones_retry() {
    for (reason, settles) in [
        (WorkerReason::NoVideoFound, true),
        (WorkerReason::StaticVideo, true),
        (WorkerReason::SourceRateLimited, false),
        (WorkerReason::DownloadFailed, false),
        (WorkerReason::AudioNotFound, false),
    ] {
        let verdict = result::judge(&done("11", 1, Some(reason)));
        assert_eq!(
            matches!(verdict, Ok(Verdict::Settle(_))),
            settles,
            "{}",
            reason.as_str()
        );
    }
}

mod sources {
    use backend_contracts::pipeline::VideoCandidateKind;

    use super::super::sources::{Failure, Sources, Wanted};
    use crate::handlers::video_accounts::SpotifyKey;
    use crate::handlers::video_accounts::check::mock::{Answer, Mock};

    fn wanted() -> Wanted {
        Wanted {
            artist: "Central Cee".to_owned(),
            title: "Doja".to_owned(),
            isrc: Some("GBUM72300001".to_owned()),
            duration_ms: Some(96_000),
        }
    }

    fn spotify(mock: &std::sync::Arc<Mock>) -> Sources<std::sync::Arc<Mock>> {
        mock.route(
            "https://open.spotify.com/api/server-time",
            Answer::Body(r#"{"serverTime":1700000000}"#.to_owned()),
        );
        mock.route(
            "https://open.spotify.com/api/token",
            Answer::Body(r#"{"accessToken":"tok","isAnonymous":false}"#.to_owned()),
        );
        Sources::new(mock.clone(), SpotifyKey::new(61, &[41, 41, 41]))
    }

    #[tokio::test]
    async fn an_isrc_hit_with_a_canvas_becomes_a_trusted_video_url() {
        let mock = Mock::new();
        let sources = spotify(&mock);
        mock.route(
            "https://api.spotify.com/v1/search",
            Answer::Body(
                r#"{"tracks":{"items":[{"uri":"spotify:track:abc","name":"Doja","duration_ms":96000,"artists":[{"name":"Central Cee"}]}]}}"#
                    .to_owned(),
            ),
        );
        mock.route(
            "https://api-partner.spotify.com/pathfinder/v2/query",
            Answer::Body(
                r#"{"data":{"trackUnion":{"canvas":{"url":"https://canvaz.scdn.co/upload/a.cnvs.mp4"}}}}"#
                    .to_owned(),
            ),
        );

        let found = sources.canvas("dc", &wanted()).await.unwrap().unwrap();

        assert_eq!(found.value, "https://canvaz.scdn.co/upload/a.cnvs.mp4");
        assert!(found.trusted);
        assert_eq!(found.kind, VideoCandidateKind::Url);
        assert_eq!(found.source_ref.as_deref(), Some("spotify:track:abc"));
    }

    #[tokio::test]
    async fn a_text_match_with_another_title_is_not_a_canvas_candidate() {
        let mock = Mock::new();
        let sources = spotify(&mock);
        let mut other = wanted();
        other.isrc = None;
        mock.route(
            "https://api.spotify.com/v1/search",
            Answer::Body(
                r#"{"tracks":{"items":[{"uri":"spotify:track:zzz","name":"Another Song Entirely","duration_ms":96000,"artists":[{"name":"Central Cee"}]}]}}"#
                    .to_owned(),
            ),
        );

        assert_eq!(sources.canvas("dc", &other).await, Ok(None));
    }

    #[tokio::test]
    async fn a_429_pauses_and_a_401_rejects_the_account() {
        for (status, paused) in [(429_u16, true), (401, false)] {
            let mock = Mock::new();
            let sources = spotify(&mock);
            mock.route("https://api.spotify.com/v1/search", Answer::Status(status));

            let failure = sources.canvas("dc", &wanted()).await.unwrap_err();

            assert_eq!(matches!(failure, Failure::Paused(_)), paused, "{failure:?}");
            assert_eq!(
                matches!(failure, Failure::Rejected(_)),
                !paused,
                "{failure:?}"
            );
        }
    }

    #[tokio::test]
    async fn yandex_asks_from_a_russian_exit_and_takes_the_supplement_clips() {
        let mock = Mock::new();
        let sources = Sources::new(mock.clone(), SpotifyKey::new(61, &[41, 41, 41]));
        mock.route(
            "https://api.music.yandex.net/search",
            Answer::Body(
                r#"{"result":{"tracks":{"results":[{"id":77,"title":"Doja","durationMs":96500,"artists":[{"name":"Central Cee"}]}]}}}"#
                    .to_owned(),
            ),
        );
        mock.route(
            "https://api.music.yandex.net/tracks/77/supplement",
            Answer::Body(
                r#"{"result":{"videos":[{"url":"https://example.test/v/1"}]}}"#.to_owned(),
            ),
        );

        let clips = sources.clips("tok", &wanted()).await.unwrap();

        assert_eq!(clips.len(), 1);
        let requests = mock.requests();
        assert!(
            requests
                .iter()
                .all(|(_, headers)| headers["x-egress-tag"] == "geo-ru")
        );
    }
}

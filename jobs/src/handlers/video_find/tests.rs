use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use backend_contracts::admin_video::VideoSource;
use backend_contracts::pipeline::{
    Producer, VideoCandidateKind, VideoClip, VideoFindResult, VideoKind, VideoPart,
};
use backend_contracts::reasons::{WorkerReason, WorkerStatus};
use base64::Engine as _;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use super::VideoFind;
use super::queue;
use super::result::{self, Verdict};
use super::sources::{Failure, Lead};
use super::stage::{Staged, StagedCandidate, temp_key};
use super::ytdlp::{Exit, Found, Tube};
use crate::config::VideoFindConfig;
use crate::handlers::video_accounts::check::mock::{Answer, Mock};
use crate::handlers::video_accounts::{Seal, SpotifyKey, store};
use crate::objects::{Blobs, Listed, ObjectsError};

const PAUSE: Duration = Duration::from_secs(3_600);

fn config() -> VideoFindConfig {
    VideoFindConfig {
        in_flight: 2,
        tick: Duration::from_secs(30),
        timeout: Duration::from_secs(4_200),
        account_pause: PAUSE,
        source_gap: Duration::ZERO,
        youtube_pause: Duration::ZERO,
        retry_base: Duration::from_secs(900),
        max_attempts: 3,
        none_recheck: Duration::from_secs(30 * 86_400),
        static_recheck: Duration::from_secs(90 * 86_400),
        max_candidates: 8,
        download_timeout: Duration::from_secs(10),
        max_file_mib: 96,
        temp_ttl: Duration::from_secs(3 * 86_400),
        sweep_every: Duration::from_secs(3_600),
        ytdlp: "yt-dlp".into(),
        proxy_list: None,
        work_dir: std::env::temp_dir().join("video-find-tests"),
        objects: None,
    }
}

type Files = BTreeMap<String, (Vec<u8>, SystemTime)>;

#[derive(Clone, Default)]
struct Memory {
    files: Arc<Mutex<Files>>,
}

impl Memory {
    fn keys(&self) -> Vec<String> {
        self.files.lock().unwrap().keys().cloned().collect()
    }

    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.files
            .lock()
            .unwrap()
            .get(key)
            .map(|(bytes, _)| bytes.clone())
    }

    fn put_at(&self, key: &str, bytes: &[u8], at: SystemTime) {
        self.files
            .lock()
            .unwrap()
            .insert(key.to_owned(), (bytes.to_vec(), at));
    }
}

impl Blobs for Memory {
    async fn put_file(&self, key: &str, path: &Path) -> Result<(), ObjectsError> {
        let bytes = std::fs::read(path).unwrap();
        self.put_at(key, &bytes, SystemTime::now());
        Ok(())
    }

    async fn put_bytes(&self, key: &str, bytes: Vec<u8>) -> Result<(), ObjectsError> {
        self.put_at(key, &bytes, SystemTime::now());
        Ok(())
    }

    async fn copy(&self, from: &str, to: &str) -> Result<(), ObjectsError> {
        let bytes = self.get(from).unwrap();
        self.put_at(to, &bytes, SystemTime::now());
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), ObjectsError> {
        self.files.lock().unwrap().remove(key);
        Ok(())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<Listed>, ObjectsError> {
        Ok(self
            .files
            .lock()
            .unwrap()
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .map(|(key, (_, modified))| Listed {
                key: key.clone(),
                modified: *modified,
            })
            .collect())
    }
}

#[derive(Clone, Default)]
struct Script {
    blocked: Arc<Mutex<bool>>,
}

impl Tube for Script {
    async fn search(
        &self,
        _query: &str,
        _results: usize,
        _cookies: &str,
    ) -> Result<Vec<Found>, Failure> {
        Ok(Vec::new())
    }

    async fn download(
        &self,
        url: &str,
        stem: &Path,
        cookies: Option<&str>,
        _exit: Exit,
    ) -> Result<std::path::PathBuf, Failure> {
        if *self.blocked.lock().unwrap() {
            return Err(Failure::Paused("429".to_owned()));
        }
        assert_eq!(cookies.is_some(), url.contains("youtube"));
        let path = stem.with_extension("mp4");
        std::fs::write(&path, format!("bytes of {url}")).unwrap();
        Ok(path)
    }
}

type Rig = VideoFind<Arc<Mock>, Memory, Script>;

fn seal() -> Seal {
    Seal::new(&[5; 32])
}

fn rig(pool: &PgPool) -> (Rig, Arc<Mock>, Memory, Script) {
    let mock = Mock::new();
    let blobs = Memory::default();
    let tube = Script::default();
    let video = VideoFind::new(
        pool.clone(),
        "https://storage.test/".parse().unwrap(),
        config(),
        mock.clone(),
        SpotifyKey::new(61, &[41, 41, 41]),
        blobs.clone(),
        tube.clone(),
        seal(),
    );
    (video, mock, blobs, tube)
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
        "INSERT INTO video_accounts (source, name, secret, status) VALUES ($1, $2, $3, 'ok')",
    )
    .bind(source)
    .bind(name)
    .bind(seal().seal(name, "cookie")?)
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

async fn staged_claim(
    pool: &PgPool,
    blobs: &Memory,
    id: &str,
    count: u32,
) -> anyhow::Result<queue::Claimed> {
    let claim = claim(pool, 1).await?.remove(0);
    let mut staged = Staged::default();
    for index in 0..count {
        let key = temp_key(id, claim.attempt, index);
        blobs.put_at(&key, format!("temp {index}").as_bytes(), SystemTime::now());
        staged.candidates.push(StagedCandidate {
            index,
            source: VideoSource::Youtube,
            kind: VideoCandidateKind::Video,
            source_ref: Some(format!("ref{index}")),
            key,
        });
    }
    let marked = queue::stage(pool, &claim, &staged)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    assert!(marked);
    Ok(claim)
}

fn producer() -> Producer {
    Producer {
        worker_id: "sc-worker1".to_owned(),
        build: "test".to_owned(),
        models: BTreeMap::new(),
        sync_version: None,
    }
}

fn done(id: &str, attempt: i64, reason: Option<WorkerReason>) -> VideoFindResult {
    let found = reason.is_none();
    VideoFindResult {
        sc_track_id: id.to_owned(),
        attempt,
        status: reason.map_or(WorkerStatus::Ok, WorkerReason::status),
        reason,
        detail: None,
        producer: producer(),
        kind: found.then_some(VideoKind::Loop),
        uncertain: found.then_some(false),
        candidate: found.then_some(1),
        offset_ms: found.then_some(3_839),
        duration_ms: found.then_some(96_000),
        looped: found.then_some(true),
        scores: found.then(|| BTreeMap::from([("ber".to_owned(), 0.02)])),
        clip: None,
        part: None,
    }
}

fn part_message(
    id: &str,
    attempt: i64,
    file: &[u8],
    parts: &[&[u8]],
    index: usize,
) -> VideoFindResult {
    let digest = Sha256::digest(file);
    let mut result = done(id, attempt, None);
    result.kind = Some(VideoKind::Clip);
    result.looped = Some(false);
    result.clip = Some(VideoClip {
        bytes: file.len() as u64,
        sha256: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
        parts: parts.len() as u32,
    });
    result.part = Some(VideoPart {
        index: index as u32,
        data: base64::engine::general_purpose::STANDARD.encode(parts[index]),
    });
    result
}

async fn finish(video: &Rig, result: &VideoFindResult) -> anyhow::Result<()> {
    video
        .finish(result.clone())
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

async fn stored(
    pool: &PgPool,
    id: &str,
) -> anyhow::Result<Option<(String, String, Option<String>, String, bool)>> {
    Ok(sqlx::query_as(
        "SELECT kind, source, source_ref, s3_key, loop FROM track_videos WHERE sc_track_id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
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
async fn accounts_rotate_and_a_paused_or_dead_one_is_skipped(pool: PgPool) -> anyhow::Result<()> {
    account(&pool, "yt1", "youtube").await?;
    account(&pool, "yt2", "youtube").await?;
    account(&pool, "yt3", "youtube").await?;
    account(&pool, "sp1", "spotify").await?;
    let lease = |source: &'static str| {
        let pool = pool.clone();
        async move { store::lease(&pool, &seal(), source).await }
    };

    let first = lease("youtube").await?;
    let second = lease("youtube").await?;
    store::pause(&pool, "yt2", PAUSE, "429").await?;
    store::kill(&pool, "yt3", "dead").await?;
    let third = lease("youtube").await?;

    assert_eq!(first, Some(("yt1".to_owned(), "cookie".to_owned())));
    assert_eq!(second.map(|leased| leased.0).as_deref(), Some("yt2"));
    assert_eq!(third.map(|leased| leased.0).as_deref(), Some("yt1"));
    assert_eq!(
        lease("spotify").await?.map(|leased| leased.0).as_deref(),
        Some("sp1")
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn candidates_are_fetched_into_temp_keys_and_a_paused_youtube_account_stops_the_rest(
    pool: PgPool,
) -> anyhow::Result<()> {
    account(&pool, "yt1", "youtube").await?;
    star_track(&pool, "11", 1, 1).await?;
    let (video, mock, blobs, tube) = rig(&pool);
    mock.route(
        "https://canvaz.scdn.co/a.mp4",
        Answer::Body("canvas bytes".to_owned()),
    );
    let claim = claim(&pool, 1).await?.remove(0);
    let youtube = video.lease_youtube().await?.unwrap();
    let leads = vec![
        Lead {
            source: VideoSource::Spotify,
            kind: VideoCandidateKind::Canvas,
            url: "https://canvaz.scdn.co/a.mp4".to_owned(),
            source_ref: Some("spotify:track:abc".to_owned()),
        },
        Lead {
            source: VideoSource::Youtube,
            kind: VideoCandidateKind::Video,
            url: "https://www.youtube.com/watch?v=aaa".to_owned(),
            source_ref: Some("aaa".to_owned()),
        },
    ];

    let staged = video.bring_all(&claim, leads, &youtube).await.unwrap();

    assert_eq!(
        blobs.keys(),
        ["video-tmp/11-1-0.mp4", "video-tmp/11-1-1.mp4"]
    );
    assert_eq!(
        blobs.get("video-tmp/11-1-0.mp4").as_deref(),
        Some(&b"canvas bytes"[..])
    );
    assert_eq!(
        blobs.get("video-tmp/11-1-1.mp4").as_deref(),
        Some(&b"bytes of https://www.youtube.com/watch?v=aaa"[..])
    );
    assert_eq!(staged.candidates.len(), 2);
    assert_eq!(staged.candidates[1].source_ref.as_deref(), Some("aaa"));

    *tube.blocked.lock().unwrap() = true;
    let more = vec![Lead {
        source: VideoSource::Youtube,
        kind: VideoCandidateKind::Video,
        url: "https://www.youtube.com/watch?v=bbb".to_owned(),
        source_ref: Some("bbb".to_owned()),
    }];
    let none = video.bring_all(&claim, more, &youtube).await.unwrap();
    let paused: (bool,) =
        sqlx::query_as("SELECT paused_until > now() FROM video_accounts WHERE name = 'yt1'")
            .fetch_one(&pool)
            .await?;
    assert!(none.candidates.is_empty());
    assert!(paused.0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_loop_is_promoted_from_its_temp_file_and_every_temp_file_goes(
    pool: PgPool,
) -> anyhow::Result<()> {
    star_track(&pool, "11", 1, 1).await?;
    let (video, _, blobs, _) = rig(&pool);
    let claim = staged_claim(&pool, &blobs, "11", 3).await?;

    finish(&video, &done("11", 1, None)).await?;

    assert_eq!(claim.attempt, 1);
    assert_eq!(blobs.keys(), ["video/11-1.mp4"]);
    assert_eq!(blobs.get("video/11-1.mp4").as_deref(), Some(&b"temp 1"[..]));
    assert_eq!(
        stored(&pool, "11").await?,
        Some((
            "loop".to_owned(),
            "youtube".to_owned(),
            Some("ref1".to_owned()),
            "video/11-1.mp4".to_owned(),
            true
        ))
    );
    assert_eq!(stage(&pool, "11").await?.0, "found");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_clip_arrives_in_parts_in_any_order_and_is_written_once_whole(
    pool: PgPool,
) -> anyhow::Result<()> {
    star_track(&pool, "11", 1, 1).await?;
    let (video, _, blobs, _) = rig(&pool);
    staged_claim(&pool, &blobs, "11", 2).await?;
    let file = b"the whole cut segment";
    let parts: [&[u8]; 3] = [b"the whole", b" cut ", b"segment"];

    finish(&video, &part_message("11", 1, file, &parts, 2)).await?;
    finish(&video, &part_message("11", 1, file, &parts, 0)).await?;
    assert_eq!(stored(&pool, "11").await?, None);
    assert_eq!(stage(&pool, "11").await?.0, "dispatched");
    finish(&video, &part_message("11", 1, file, &parts, 1)).await?;

    assert_eq!(blobs.keys(), ["video/11-1.mp4"]);
    assert_eq!(blobs.get("video/11-1.mp4").as_deref(), Some(&file[..]));
    let row = stored(&pool, "11").await?.unwrap();
    assert_eq!((row.0.as_str(), row.4), ("clip", false));
    assert_eq!(stage(&pool, "11").await?.0, "found");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_clip_whose_checksum_fails_is_not_stored(pool: PgPool) -> anyhow::Result<()> {
    star_track(&pool, "11", 1, 1).await?;
    let (video, _, blobs, _) = rig(&pool);
    staged_claim(&pool, &blobs, "11", 2).await?;
    let parts: [&[u8]; 2] = [b"abc", b"def"];

    finish(&video, &part_message("11", 1, b"abcdef", &parts, 0)).await?;
    let corrupted = part_message("11", 1, b"abcdef", &[b"abc", b"xyz"], 1);

    assert!(finish(&video, &corrupted).await.is_err());
    assert_eq!(stored(&pool, "11").await?, None);
    assert!(blobs.get("video/11-1.mp4").is_none());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_uncertain_clip_is_held_for_the_vlm_with_its_temp_file_and_the_rest_go(
    pool: PgPool,
) -> anyhow::Result<()> {
    star_track(&pool, "11", 1, 1).await?;
    let (video, _, blobs, _) = rig(&pool);
    staged_claim(&pool, &blobs, "11", 3).await?;
    let mut uncertain = done("11", 1, None);
    uncertain.uncertain = Some(true);

    finish(&video, &uncertain).await?;

    let held: (Option<String>,) = sqlx::query_as(
        "SELECT payload ->> 's3_key' FROM track_video_dispatch WHERE sc_track_id = '11'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(stored(&pool, "11").await?, None);
    assert_eq!(stage(&pool, "11").await?.0, "pending_vlm");
    assert_eq!(held.0.as_deref(), Some("video-tmp/11-1-1.mp4"));
    assert_eq!(blobs.keys(), ["video-tmp/11-1-1.mp4"]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn nothing_found_waits_a_month_and_a_static_video_a_quarter_and_temp_files_go(
    pool: PgPool,
) -> anyhow::Result<()> {
    star_track(&pool, "11", 2, 1).await?;
    star_track(&pool, "12", 1, 1).await?;
    let (video, _, blobs, _) = rig(&pool);
    staged_claim(&pool, &blobs, "11", 2).await?;
    staged_claim(&pool, &blobs, "12", 2).await?;

    finish(&video, &done("11", 1, Some(WorkerReason::NoVideoFound))).await?;
    finish(&video, &done("12", 1, Some(WorkerReason::StaticVideo))).await?;

    let waits: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT sc_track_id, stage, (extract(epoch FROM next_at - now()) / 86400)::bigint
         FROM track_video_dispatch ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(waits[0], ("11".to_owned(), "none".to_owned(), 30));
    assert_eq!(waits[1], ("12".to_owned(), "static".to_owned(), 90));
    assert!(blobs.keys().is_empty());
    assert!(claim(&pool, 5).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn gone_inputs_retry_the_track_and_the_temp_files_go(pool: PgPool) -> anyhow::Result<()> {
    star_track(&pool, "11", 1, 1).await?;
    let (video, _, blobs, _) = rig(&pool);
    staged_claim(&pool, &blobs, "11", 2).await?;

    finish(&video, &done("11", 1, Some(WorkerReason::InputsGone))).await?;

    let row = stage(&pool, "11").await?;
    assert_eq!(
        (row.0.as_str(), row.2.as_deref()),
        ("retry", Some("inputs_gone"))
    );
    assert!(blobs.keys().is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_stale_answer_changes_nothing_and_keeps_the_new_attempt_files(
    pool: PgPool,
) -> anyhow::Result<()> {
    star_track(&pool, "11", 1, 1).await?;
    let (video, _, blobs, _) = rig(&pool);
    staged_claim(&pool, &blobs, "11", 2).await?;

    finish(&video, &done("11", 2, None)).await?;

    assert_eq!(stored(&pool, "11").await?, None);
    assert_eq!(stage(&pool, "11").await?.0, "dispatched");
    assert_eq!(blobs.keys().len(), 2);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_transient_failure_backs_off_and_gives_up_at_the_attempt_cap(
    pool: PgPool,
) -> anyhow::Result<()> {
    star_track(&pool, "11", 1, 1).await?;
    let (video, _, blobs, _) = rig(&pool);
    for attempt in 1..=3 {
        sqlx::query("UPDATE track_video_dispatch SET next_at = now() WHERE sc_track_id = '11'")
            .execute(&pool)
            .await?;
        let claimed = staged_claim(&pool, &blobs, "11", 1).await?;
        assert_eq!(claimed.attempt, attempt);
        finish(
            &video,
            &done("11", i64::from(attempt), Some(WorkerReason::DownloadFailed)),
        )
        .await?;
        let expected = if attempt < 3 { "retry" } else { "none" };
        assert_eq!(stage(&pool, "11").await?.0, expected);
    }
    assert!(blobs.keys().is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_sweep_deletes_only_temp_files_past_their_three_days(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (video, _, blobs, _) = rig(&pool);
    let old = SystemTime::now() - Duration::from_secs(4 * 86_400);
    blobs.put_at("video-tmp/9-1-0.mp4", b"x", old);
    blobs.put_at("video-tmp/9-2-0.mp4", b"x", SystemTime::now());
    blobs.put_at("video/9-1.mp4", b"x", old);

    let removed = video.expire_once().await?;

    assert_eq!(removed, 1);
    assert_eq!(blobs.keys(), ["video-tmp/9-2-0.mp4", "video/9-1.mp4"]);
    Ok(())
}

#[test]
fn a_result_naming_no_candidate_or_no_window_is_refused() {
    let mut nameless = done("11", 1, None);
    nameless.candidate = None;
    assert!(result::judge(&nameless).is_err());
    let mut windowless = done("11", 1, None);
    windowless.duration_ms = None;
    assert!(result::judge(&windowless).is_err());
    let mut clip_without_file = done("11", 1, None);
    clip_without_file.kind = Some(VideoKind::Clip);
    assert!(result::judge(&clip_without_file).is_err());
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
        (WorkerReason::InputsGone, false),
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
    async fn an_isrc_hit_with_a_canvas_becomes_a_canvas_lead() {
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

        assert_eq!(found.url, "https://canvaz.scdn.co/upload/a.cnvs.mp4");
        assert_eq!(found.kind, VideoCandidateKind::Canvas);
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

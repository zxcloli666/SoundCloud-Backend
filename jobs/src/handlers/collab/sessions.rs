use std::collections::HashSet;

use backend_contracts::pipeline::COLLAB_DATASET_VERSION;
use chrono::Utc;
use futures::TryStreamExt;
use sqlx::PgPool;
use tempfile::TempPath;
use tokio::io::{AsyncWriteExt, BufWriter};
use tracing::warn;

use crate::queue::{JobError, JobResult};

const HISTORY_DAYS: i64 = 90;
const SESSION_GAP_MILLIS: i64 = 30 * 60 * 1_000;
const MIN_SESSION_LENGTH: usize = 2;
const MAX_SESSION_LENGTH: usize = 200;
const EVENT_TYPES: &[&str] = &["like", "playlist_add", "full_play"];
const CLOSING: &[u8] = b"]}";

pub(super) struct TrainingDataset {
    path: TempPath,
    pub session_count: usize,
    pub event_count: usize,
    pub truncated: bool,
}

impl TrainingDataset {
    pub async fn open(&self) -> std::io::Result<tokio::fs::File> {
        tokio::fs::File::open(&self.path).await
    }
}

pub(super) async fn build(pool: &PgPool, max_bytes: usize) -> JobResult<TrainingDataset> {
    let temp = tempfile::NamedTempFile::new().map_err(JobError::retryable)?;
    let path = temp.into_temp_path();
    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&path)
        .await
        .map_err(JobError::retryable)?;
    let mut writer = DatasetWriter::new(path, file, max_bytes).await?;
    let since = Utc::now().naive_utc() - chrono::Duration::days(HISTORY_DAYS);
    let event_types = EVENT_TYPES
        .iter()
        .map(|event_type| (*event_type).to_owned())
        .collect::<Vec<_>>();
    let mut rows =
        sqlx::query_file!("queries/collab/build_sessions.sql", since, &event_types).fetch(pool);
    let mut sessions = SessionBuilder::default();
    let mut event_count = 0usize;

    while let Some(row) = rows.try_next().await.map_err(JobError::retryable)? {
        event_count += 1;
        let Ok(track_id) = row.sc_track_id.parse::<u64>() else {
            continue;
        };
        let finished = sessions.push(
            &row.sc_user_id,
            track_id,
            row.created_at.and_utc().timestamp_millis(),
        );
        if let Some(session) = finished
            && !writer.write_session(&session).await?
        {
            break;
        }
    }
    drop(rows);
    if !writer.truncated
        && let Some(session) = sessions.finish()
    {
        writer.write_session(&session).await?;
    }
    let dataset = writer.finish(event_count).await?;
    if dataset.truncated {
        warn!(
            sessions = dataset.session_count,
            events = dataset.event_count,
            max_bytes,
            "collab dataset reached its byte limit; the least recently active users are left out"
        );
    }
    Ok(dataset)
}

#[derive(Default)]
struct SessionBuilder {
    current_user: Option<String>,
    current_time_millis: i64,
    current_tracks: Vec<u64>,
    current_seen: HashSet<u64>,
}

impl SessionBuilder {
    fn push(&mut self, user_id: &str, track_id: u64, created_at_millis: i64) -> Option<Vec<u64>> {
        let same_user = self.current_user.as_deref() == Some(user_id);
        let inside_session = same_user
            && created_at_millis.saturating_sub(self.current_time_millis) <= SESSION_GAP_MILLIS;
        let finished = (!inside_session).then(|| self.take()).flatten();
        if !same_user {
            self.current_user = Some(user_id.to_owned());
        }
        self.current_time_millis = created_at_millis;
        if self.current_tracks.len() < MAX_SESSION_LENGTH && self.current_seen.insert(track_id) {
            self.current_tracks.push(track_id);
        }
        finished
    }

    fn finish(mut self) -> Option<Vec<u64>> {
        self.take()
    }

    fn take(&mut self) -> Option<Vec<u64>> {
        self.current_seen.clear();
        if self.current_tracks.len() >= MIN_SESSION_LENGTH {
            Some(std::mem::take(&mut self.current_tracks))
        } else {
            self.current_tracks.clear();
            None
        }
    }
}

struct DatasetWriter {
    path: TempPath,
    writer: BufWriter<tokio::fs::File>,
    max_bytes: usize,
    written_bytes: usize,
    session_count: usize,
    truncated: bool,
}

impl DatasetWriter {
    async fn new(path: TempPath, file: tokio::fs::File, max_bytes: usize) -> JobResult<Self> {
        let mut writer = Self {
            path,
            writer: BufWriter::new(file),
            max_bytes,
            written_bytes: 0,
            session_count: 0,
            truncated: false,
        };
        let opening = format!("{{\"version\":{COLLAB_DATASET_VERSION},\"sessions\":[");
        if opening.len() + CLOSING.len() > max_bytes {
            return Err(JobError::permanent(anyhow::anyhow!(
                "collab dataset limit of {max_bytes} bytes cannot hold an empty dataset"
            )));
        }
        writer.write(opening.as_bytes()).await?;
        Ok(writer)
    }

    async fn write_session(&mut self, session: &[u64]) -> JobResult<bool> {
        if self.truncated {
            return Ok(false);
        }
        let payload = serde_json::to_vec(session).map_err(JobError::permanent)?;
        let separator: &[u8] = if self.session_count > 0 { b"," } else { b"" };
        let needed = separator.len() + payload.len() + CLOSING.len();
        if self.written_bytes.saturating_add(needed) > self.max_bytes {
            self.truncated = true;
            return Ok(false);
        }
        self.write(separator).await?;
        self.write(&payload).await?;
        self.session_count += 1;
        Ok(true)
    }

    async fn finish(mut self, event_count: usize) -> JobResult<TrainingDataset> {
        self.write(CLOSING).await?;
        self.writer.flush().await.map_err(JobError::retryable)?;
        Ok(TrainingDataset {
            path: self.path,
            session_count: self.session_count,
            event_count,
            truncated: self.truncated,
        })
    }

    async fn write(&mut self, bytes: &[u8]) -> JobResult {
        let total = self.written_bytes.saturating_add(bytes.len());
        if total > self.max_bytes {
            return Err(JobError::permanent(anyhow::anyhow!(
                "collab dataset exceeds its {} byte limit",
                self.max_bytes
            )));
        }
        self.writer
            .write_all(bytes)
            .await
            .map_err(JobError::retryable)?;
        self.written_bytes = total;
        Ok(())
    }
}

#[cfg(test)]
fn collect_sessions(events: &[(&str, u64, i64)]) -> Vec<Vec<u64>> {
    let mut builder = SessionBuilder::default();
    let mut sessions = Vec::new();
    for (user_id, track_id, created_at) in events {
        if let Some(session) = builder.push(user_id, *track_id, *created_at) {
            sessions.push(session);
        }
    }
    if let Some(session) = builder.finish() {
        sessions.push(session);
    }
    sessions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_users_and_idle_gaps() {
        let sessions = collect_sessions(&[
            ("one", 1, 0),
            ("one", 2, 1),
            ("one", 3, SESSION_GAP_MILLIS + 2),
            ("one", 4, SESSION_GAP_MILLIS + 3),
            ("two", 5, SESSION_GAP_MILLIS + 4),
            ("two", 6, SESSION_GAP_MILLIS + 5),
        ]);

        assert_eq!(sessions, vec![vec![1, 2], vec![3, 4], vec![5, 6]]);
    }

    #[test]
    fn keeps_the_first_occurrence_and_caps_long_sessions() {
        let mut events = vec![("one", 1, 0), ("one", 1, 1)];
        for track_id in 2..=250 {
            events.push(("one", track_id, i64::try_from(track_id).unwrap_or_default()));
        }

        let sessions = collect_sessions(&events);
        assert_eq!(sessions.len(), 1);
        let session = sessions.first().map(Vec::as_slice).unwrap_or_default();
        assert_eq!(session.len(), MAX_SESSION_LENGTH);
        assert_eq!(session.get(..2), Some(&[1, 2][..]));
    }

    async fn writer_limited_to(max_bytes: usize) -> anyhow::Result<DatasetWriter> {
        let temp = tempfile::NamedTempFile::new()?;
        let path = temp.into_temp_path();
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .await?;
        Ok(DatasetWriter::new(path, file, max_bytes).await?)
    }

    #[tokio::test]
    async fn a_dataset_over_its_limit_is_truncated_instead_of_failing() -> anyhow::Result<()> {
        let opening = r#"{"version":2,"sessions":["#.len();
        let mut writer = writer_limited_to(opening + "[1,2],[3,4]".len() + 2).await?;

        assert!(writer.write_session(&[1, 2]).await?);
        assert!(writer.write_session(&[3, 4]).await?);
        assert!(!writer.write_session(&[5, 6]).await?);
        assert!(!writer.write_session(&[7]).await?);
        let dataset = writer.finish(7).await?;

        let written = tokio::fs::read(&dataset.path).await?;
        let envelope: serde_json::Value = serde_json::from_slice(&written)?;
        assert_eq!(
            envelope,
            serde_json::json!({ "version": 2, "sessions": [[1, 2], [3, 4]] })
        );
        assert!(dataset.truncated);
        assert_eq!(dataset.session_count, 2);
        Ok(())
    }

    #[sqlx::test(migrations = "../api/migrations")]
    async fn a_truncated_dataset_keeps_the_most_recently_active_listeners(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO user_events (sc_user_id, sc_track_id, event_type, weight, created_at)
             VALUES ('aaa', '1', 'like', 1, now() - interval '10 days'),
                    ('aaa', '2', 'like', 1, now() - interval '10 days' + interval '1 minute'),
                    ('zzz', '3', 'like', 1, now() - interval '1 day'),
                    ('zzz', '4', 'like', 1, now() - interval '1 day' + interval '1 minute')",
        )
        .execute(&pool)
        .await?;
        let opening = r#"{"version":2,"sessions":["#.len();

        let dataset = build(&pool, opening + "[3,4]".len() + CLOSING.len()).await?;

        let written = tokio::fs::read(&dataset.path).await?;
        let envelope: serde_json::Value = serde_json::from_slice(&written)?;
        assert_eq!(
            envelope,
            serde_json::json!({ "version": 2, "sessions": [[3, 4]] })
        );
        assert!(dataset.truncated);
        Ok(())
    }

    #[tokio::test]
    async fn a_limit_too_small_for_an_empty_dataset_is_a_configuration_error() {
        assert!(writer_limited_to(4).await.is_err());
    }

    #[tokio::test]
    async fn the_dataset_is_the_versioned_envelope_the_worker_reads() -> anyhow::Result<()> {
        let mut writer = writer_limited_to(1024).await?;
        writer.write_session(&[1, 2]).await?;
        writer.write_session(&[3, 4, 5]).await?;
        let dataset = writer.finish(5).await?;
        assert!(!dataset.truncated);

        let written = tokio::fs::read(&dataset.path).await?;
        let envelope: serde_json::Value = serde_json::from_slice(&written)?;

        assert_eq!(
            envelope,
            serde_json::json!({ "version": 2, "sessions": [[1, 2], [3, 4, 5]] })
        );
        assert_eq!(dataset.session_count, 2);
        Ok(())
    }

    #[test]
    fn a_skip_is_not_a_signal_that_two_tracks_belong_together() {
        assert!(!EVENT_TYPES.contains(&"skip"));
        assert_eq!(EVENT_TYPES, &["like", "playlist_add", "full_play"]);
    }

    #[test]
    fn temp_paths_are_files() -> anyhow::Result<()> {
        let temp = tempfile::NamedTempFile::new()?;
        assert!(temp.path().is_file());
        Ok(())
    }
}

use std::collections::HashMap;

use serde_json::{Map, Value};
use sqlx::PgPool;

use crate::common::sc_ids::normalize_sc_track_id;
use crate::error::AppResult;

const STALE_SECS: i64 = 300;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Counters {
    play_count: Option<i64>,
    likes_count: Option<i64>,
    reposts_count: Option<i64>,
    comment_count: Option<i64>,
}

impl Counters {
    fn is_empty(&self) -> bool {
        self.play_count.is_none()
            && self.likes_count.is_none()
            && self.reposts_count.is_none()
            && self.comment_count.is_none()
    }

    fn merged_with(&self, incoming: &Counters) -> Counters {
        Counters {
            play_count: incoming.play_count.or(self.play_count),
            likes_count: incoming.likes_count.or(self.likes_count),
            reposts_count: incoming.reposts_count.or(self.reposts_count),
            comment_count: incoming.comment_count.or(self.comment_count),
        }
    }
}

fn read_i64_from(obj: &Map<String, Value>, key: &str) -> Option<i64> {
    let n = obj.get(key)?;
    if n.is_null() {
        return None;
    }
    n.as_i64().or_else(|| n.as_u64().map(|u| u as i64))
}

fn read_counters_obj(obj: &Map<String, Value>) -> Counters {
    Counters {
        play_count: read_i64_from(obj, "playback_count")
            .or_else(|| read_i64_from(obj, "play_count")),
        likes_count: read_i64_from(obj, "likes_count")
            .or_else(|| read_i64_from(obj, "favoritings_count")),
        reposts_count: read_i64_from(obj, "reposts_count"),
        comment_count: read_i64_from(obj, "comment_count"),
    }
}

fn read_counters(v: &Value) -> Counters {
    v.as_object().map(read_counters_obj).unwrap_or_default()
}

pub async fn sync(pg: &PgPool, tracks: &mut [Value]) -> AppResult<()> {
    if tracks.is_empty() {
        return Ok(());
    }

    let mut fresh: HashMap<String, Counters> = HashMap::new();
    let mut sc_ids: Vec<String> = Vec::with_capacity(tracks.len());
    for t in tracks.iter() {
        let Some(urn) = t.get("urn").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(sc_id) = normalize_sc_track_id(urn) else {
            continue;
        };
        let c = read_counters(t);
        if !c.is_empty() {
            fresh.insert(sc_id.clone(), c);
        }
        sc_ids.push(sc_id);
    }
    if sc_ids.is_empty() {
        return Ok(());
    }

    let rows = sqlx::query_file!("queries/tracks/counters/select_counters.sql", &sc_ids,)
        .fetch_all(pg)
        .await?;
    let now = chrono::Utc::now();
    let mut stored: HashMap<String, (Counters, i64)> = rows
        .into_iter()
        .map(|row| {
            let age = (now - row.fetched_at).num_seconds();
            (
                row.sc_track_id,
                (
                    Counters {
                        play_count: row.play_count,
                        likes_count: row.likes_count,
                        reposts_count: row.reposts_count,
                        comment_count: row.comment_count,
                    },
                    age,
                ),
            )
        })
        .collect();

    let mut pending: Vec<(String, Counters)> = Vec::new();
    for (sc_id, incoming) in &fresh {
        let merged = match stored.get(sc_id) {
            Some((current, _)) => {
                let merged = current.merged_with(incoming);
                if merged == *current {
                    continue;
                }
                merged
            }
            None => *incoming,
        };
        pending.push((sc_id.clone(), merged));
    }

    if !pending.is_empty() {
        pending.sort_by(|a, b| a.0.cmp(&b.0));

        let mut ids: Vec<String> = Vec::with_capacity(pending.len());
        let mut play: Vec<Option<i64>> = Vec::with_capacity(pending.len());
        let mut likes: Vec<Option<i64>> = Vec::with_capacity(pending.len());
        let mut reposts: Vec<Option<i64>> = Vec::with_capacity(pending.len());
        let mut comments: Vec<Option<i64>> = Vec::with_capacity(pending.len());
        for (id, c) in &pending {
            ids.push(id.clone());
            play.push(c.play_count);
            likes.push(c.likes_count);
            reposts.push(c.reposts_count);
            comments.push(c.comment_count);
        }
        sqlx::query(
            "INSERT INTO sc_track_counters (sc_track_id, play_count, likes_count, reposts_count, comment_count, fetched_at) \
             SELECT u.id, u.p, u.l, u.r, u.c, now() \
             FROM UNNEST($1::text[], $2::bigint[], $3::bigint[], $4::bigint[], $5::bigint[]) AS u(id, p, l, r, c) \
             ORDER BY u.id \
             ON CONFLICT (sc_track_id) DO UPDATE SET \
                 play_count    = COALESCE(EXCLUDED.play_count, sc_track_counters.play_count), \
                 likes_count   = COALESCE(EXCLUDED.likes_count, sc_track_counters.likes_count), \
                 reposts_count = COALESCE(EXCLUDED.reposts_count, sc_track_counters.reposts_count), \
                 comment_count = COALESCE(EXCLUDED.comment_count, sc_track_counters.comment_count), \
                 fetched_at    = now() \
             WHERE sc_track_counters.play_count    IS DISTINCT FROM COALESCE(EXCLUDED.play_count,    sc_track_counters.play_count) \
                OR sc_track_counters.likes_count   IS DISTINCT FROM COALESCE(EXCLUDED.likes_count,   sc_track_counters.likes_count) \
                OR sc_track_counters.reposts_count IS DISTINCT FROM COALESCE(EXCLUDED.reposts_count, sc_track_counters.reposts_count) \
                OR sc_track_counters.comment_count IS DISTINCT FROM COALESCE(EXCLUDED.comment_count, sc_track_counters.comment_count)",
        )
        .bind(&ids)
        .bind(&play)
        .bind(&likes)
        .bind(&reposts)
        .bind(&comments)
        .execute(pg)
        .await?;
        for (id, c) in pending {
            stored.insert(id, (c, 0));
        }
    }

    if stored.is_empty() {
        return Ok(());
    }

    for t in tracks.iter_mut() {
        let Some(urn) = t.get("urn").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(sc_id) = normalize_sc_track_id(urn) else {
            continue;
        };
        let Some((c, age)) = stored.get(&sc_id) else {
            continue;
        };
        let Some(obj) = t.as_object_mut() else {
            continue;
        };
        let stale = *age > STALE_SECS;
        let already_present = read_counters_obj(obj);
        if let Some(v) = c.play_count
            && (already_present.play_count.is_none()
                || (!stale && already_present.play_count != Some(v)))
        {
            obj.insert("playback_count".into(), Value::from(v));
        }
        if let Some(v) = c.likes_count
            && (already_present.likes_count.is_none()
                || (!stale && already_present.likes_count != Some(v)))
        {
            obj.insert("likes_count".into(), Value::from(v));
            obj.insert("favoritings_count".into(), Value::from(v));
        }
        if let Some(v) = c.reposts_count
            && (already_present.reposts_count.is_none()
                || (!stale && already_present.reposts_count != Some(v)))
        {
            obj.insert("reposts_count".into(), Value::from(v));
        }
        if let Some(v) = c.comment_count
            && (already_present.comment_count.is_none()
                || (!stale && already_present.comment_count != Some(v)))
        {
            obj.insert("comment_count".into(), Value::from(v));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const URN: &str = "soundcloud:tracks:42";

    fn projection(playback: i64) -> Value {
        serde_json::json!({ "urn": URN, "playback_count": playback })
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_unchanged_sync_never_waits_on_a_writer_row_lock(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let mut seed = vec![projection(7)];
        sync(&pool, &mut seed).await?;

        let mut holder = pool.begin().await?;
        sqlx::query("SELECT 1 FROM sc_track_counters WHERE sc_track_id = '42' FOR UPDATE")
            .execute(&mut *holder)
            .await?;

        let mut again = vec![projection(7)];
        let outcome = tokio::time::timeout(Duration::from_secs(3), sync(&pool, &mut again)).await;
        holder.rollback().await?;
        outcome.map_err(|_| {
            anyhow::anyhow!(
                "a serving-path counter sync with unchanged values must not take the row lock"
            )
        })??;
        assert_eq!(again[0]["playback_count"], serde_json::json!(7));
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_changed_counter_is_still_persisted(pool: PgPool) -> anyhow::Result<()> {
        let mut seed = vec![projection(7)];
        sync(&pool, &mut seed).await?;

        let mut bumped = vec![projection(9)];
        sync(&pool, &mut bumped).await?;

        let stored: Option<i64> =
            sqlx::query_scalar("SELECT play_count FROM sc_track_counters WHERE sc_track_id = '42'")
                .fetch_one(&pool)
                .await?;
        assert_eq!(stored, Some(9));
        assert_eq!(bumped[0]["playback_count"], serde_json::json!(9));
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn stored_counters_fill_fields_the_projection_lacks(pool: PgPool) -> anyhow::Result<()> {
        let mut seed = vec![serde_json::json!({
            "urn": URN,
            "playback_count": 7,
            "likes_count": 3
        })];
        sync(&pool, &mut seed).await?;

        let mut bare = vec![serde_json::json!({ "urn": URN })];
        sync(&pool, &mut bare).await?;

        assert_eq!(bare[0]["playback_count"], serde_json::json!(7));
        assert_eq!(bare[0]["likes_count"], serde_json::json!(3));
        Ok(())
    }
}

#[cfg(test)]
#[path = "upgrade_tests.rs"]
mod tests;

use deadpool_postgres::Client;
use tracing::warn;

use super::{PgError, PgPool};

const STAR_SCAN_WINDOW: i64 = 2_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpgradeTier {
    StarLiked,
    Liked,
    Rest,
}

impl UpgradeTier {
    pub fn label(self) -> &'static str {
        match self {
            Self::StarLiked => "star_liked",
            Self::Liked => "liked",
            Self::Rest => "rest",
        }
    }
}

pub struct UpgradePick {
    pub urns: Vec<String>,
    pub waiting_seconds: i64,
}

const PICK_STAR_LIKED: &str = r#"UPDATE tracks SET hq_upgrade_last_at = now(),
       hq_upgrade_attempts = hq_upgrade_attempts + 1, updated_at = now()
   WHERE id IN (
       SELECT track.id
       FROM (
           SELECT tiered.sc_track_id, tiered.tiered_at
           FROM star_track_tiers AS tiered
           WHERE tiered.tier = 1
             AND tiered.upgrade_from = $3
           ORDER BY tiered.upgrade_tried_at NULLS FIRST, tiered.tiered_at, tiered.sc_track_id
           LIMIT $4
       ) AS star
       JOIN tracks AS track ON track.sc_track_id = star.sc_track_id
       WHERE track.hq_upgrade_pending = true
         AND track.storage_quality = $3
         AND (track.hq_upgrade_last_at IS NULL
              OR track.hq_upgrade_last_at < now() - make_interval(secs => $2))
       ORDER BY track.hq_upgrade_last_at NULLS FIRST, star.tiered_at, star.sc_track_id
       FOR UPDATE OF track SKIP LOCKED
       LIMIT $1
   )
   RETURNING urn"#;

const NEXT_STAR_LIKED: &str = r#"SELECT extract(epoch FROM now() - greatest(
           star.tiered_at,
           coalesce(track.hq_upgrade_last_at + make_interval(secs => $1),
                    track.s3_verified_at,
                    track.created_at)
       ))::bigint
   FROM (
       SELECT tiered.sc_track_id, tiered.tiered_at
       FROM star_track_tiers AS tiered
       WHERE tiered.tier = 1
         AND tiered.upgrade_from = $2
       ORDER BY tiered.upgrade_tried_at NULLS FIRST, tiered.tiered_at, tiered.sc_track_id
       LIMIT $3
   ) AS star
   JOIN tracks AS track ON track.sc_track_id = star.sc_track_id
   WHERE track.hq_upgrade_pending = true
     AND track.storage_quality = $2
     AND (track.hq_upgrade_last_at IS NULL
          OR track.hq_upgrade_last_at < now() - make_interval(secs => $1))
   ORDER BY track.hq_upgrade_last_at NULLS FIRST, star.tiered_at, star.sc_track_id
   LIMIT 1"#;

const PICK_LIKED: &str = r#"UPDATE tracks SET hq_upgrade_last_at = now(),
       hq_upgrade_attempts = hq_upgrade_attempts + 1, updated_at = now()
   WHERE id IN (
       SELECT id FROM tracks
       WHERE hq_upgrade_pending = true
         AND index_priority = 1
         AND storage_quality = $3
         AND (hq_upgrade_last_at IS NULL
              OR hq_upgrade_last_at < now() - make_interval(secs => $2))
       ORDER BY hq_upgrade_last_at NULLS FIRST
       FOR UPDATE SKIP LOCKED
       LIMIT $1
   )
   RETURNING urn"#;

const NEXT_LIKED: &str = r#"SELECT extract(epoch FROM now() - coalesce(
           hq_upgrade_last_at + make_interval(secs => $1),
           s3_verified_at,
           created_at
       ))::bigint
   FROM tracks
   WHERE hq_upgrade_pending = true
     AND index_priority = 1
     AND storage_quality = $2
     AND (hq_upgrade_last_at IS NULL
          OR hq_upgrade_last_at < now() - make_interval(secs => $1))
   ORDER BY hq_upgrade_last_at NULLS FIRST
   LIMIT 1"#;

const PICK_REST: &str = r#"UPDATE tracks SET hq_upgrade_last_at = now(),
       hq_upgrade_attempts = hq_upgrade_attempts + 1, updated_at = now()
   WHERE id IN (
       SELECT id FROM tracks
       WHERE hq_upgrade_pending = true
         AND storage_quality = $3
         AND (hq_upgrade_last_at IS NULL
              OR hq_upgrade_last_at < now() - make_interval(secs => $2))
       ORDER BY hq_upgrade_last_at NULLS FIRST
       FOR UPDATE SKIP LOCKED
       LIMIT $1
   )
   RETURNING urn"#;

const NEXT_REST: &str = r#"SELECT extract(epoch FROM now() - coalesce(
           hq_upgrade_last_at + make_interval(secs => $1),
           s3_verified_at,
           created_at
       ))::bigint
   FROM tracks
   WHERE hq_upgrade_pending = true
     AND storage_quality = $2
     AND (hq_upgrade_last_at IS NULL
          OR hq_upgrade_last_at < now() - make_interval(secs => $1))
   ORDER BY hq_upgrade_last_at NULLS FIRST
   LIMIT 1"#;

impl PgPool {
    pub async fn pick_upgrade_candidates(
        &self,
        stored: &str,
        tier: UpgradeTier,
        limit: i64,
        retry_cooldown_sec: i64,
    ) -> Result<UpgradePick, PgError> {
        let cooldown = retry_cooldown_sec as f64;
        let client = self.connection().await?;
        let urns = if limit > 0 {
            claim(&client, stored, tier, limit, cooldown).await?
        } else {
            Vec::new()
        };
        if (urns.len() as i64) < limit {
            return Ok(UpgradePick {
                urns,
                waiting_seconds: 0,
            });
        }
        let waiting_seconds = match next_waiting(&client, stored, tier, cooldown).await {
            Ok(seconds) => seconds,
            Err(e) => {
                warn!(
                    "[hq-upgrade] wait of {stored} {} unknown: {e}",
                    tier.label()
                );
                0
            }
        };
        Ok(UpgradePick {
            urns,
            waiting_seconds,
        })
    }

    pub async fn mark_hq_upgrade_failed(&self, urn: &str) -> Result<(), PgError> {
        let client = self.connection().await?;
        client
            .execute(
                "UPDATE tracks SET hq_upgrade_last_at = now(), updated_at = now() \
                 WHERE urn = $1",
                &[&urn],
            )
            .await?;
        Ok(())
    }
}

async fn claim(
    client: &Client,
    stored: &str,
    tier: UpgradeTier,
    limit: i64,
    cooldown: f64,
) -> Result<Vec<String>, tokio_postgres::Error> {
    let rows = match tier {
        UpgradeTier::StarLiked => {
            client
                .query(
                    PICK_STAR_LIKED,
                    &[&limit, &cooldown, &stored, &STAR_SCAN_WINDOW],
                )
                .await?
        }
        UpgradeTier::Liked => {
            client
                .query(PICK_LIKED, &[&limit, &cooldown, &stored])
                .await?
        }
        UpgradeTier::Rest => {
            client
                .query(PICK_REST, &[&limit, &cooldown, &stored])
                .await?
        }
    };
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

async fn next_waiting(
    client: &Client,
    stored: &str,
    tier: UpgradeTier,
    cooldown: f64,
) -> Result<i64, tokio_postgres::Error> {
    let next = match tier {
        UpgradeTier::StarLiked => {
            client
                .query_opt(NEXT_STAR_LIKED, &[&cooldown, &stored, &STAR_SCAN_WINDOW])
                .await?
        }
        UpgradeTier::Liked => client.query_opt(NEXT_LIKED, &[&cooldown, &stored]).await?,
        UpgradeTier::Rest => client.query_opt(NEXT_REST, &[&cooldown, &stored]).await?,
    };
    Ok(next.map_or(0, |row| row.get::<_, i64>(0).max(0)))
}

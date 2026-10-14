use sqlx::PgPool;

use super::LookupClaim;
use crate::handlers::star_priority::{Pass, Picks, Tier};
use crate::queue::LeasedJob;

pub(super) async fn claim(
    pool: &PgPool,
    job: &LeasedJob,
    limit: i64,
    claim_seconds: i64,
    picks: &mut Picks,
) -> Result<Vec<LookupClaim>, sqlx::Error> {
    let mut claims = Vec::new();
    for tier in Tier::ORDER {
        let left = limit.saturating_sub(i64::try_from(claims.len()).unwrap_or(i64::MAX));
        if left <= 0 {
            break;
        }
        let claimed = match tier.pass() {
            Pass::Star(stored) => {
                sqlx::query_file_as!(
                    LookupClaim,
                    "queries/lyrics/lookup_claim_star.sql",
                    job.id,
                    job.generation,
                    job.lease_id,
                    left,
                    claim_seconds,
                    stored
                )
                .fetch_all(pool)
                .await?
            }
            Pass::UpToPriority(highest) => {
                sqlx::query_file_as!(
                    LookupClaim,
                    "queries/lyrics/lookup_claim_priority.sql",
                    job.id,
                    job.generation,
                    job.lease_id,
                    left,
                    claim_seconds,
                    highest
                )
                .fetch_all(pool)
                .await?
            }
        };
        picks.record(tier.label(), claimed.iter().map(|claim| claim.due_at));
        claims.extend(claimed);
    }
    Ok(claims)
}

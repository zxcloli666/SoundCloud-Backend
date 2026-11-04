use std::collections::HashMap;

use sqlx::PgPool;

use crate::queue::{JobError, JobResult};

use super::history::{HistoryGrouper, UserHistory, event_of};

pub(super) struct RecentHistory {
    pub(super) history: UserHistory,
    pub(super) cutoff_unix_s: Option<i64>,
}

struct RecentEventRow {
    user_id: String,
    track_id: i64,
    event_code: i16,
    unix_s: Option<i64>,
    weight: f64,
    cutoff_unix_s: Option<i64>,
}

pub(super) async fn histories(
    pool: &PgPool,
    days: i32,
    variants: &[String],
    per_kind: i64,
) -> JobResult<Vec<RecentHistory>> {
    let rows = sqlx::query_file_as!(
        RecentEventRow,
        "queries/taste/recent_user_events.sql",
        days,
        variants,
        backend_contracts::SUPERLIKE_WEIGHT,
        per_kind
    )
    .fetch_all(pool)
    .await
    .map_err(JobError::retryable)?;
    Ok(grouped(rows))
}

fn grouped(rows: Vec<RecentEventRow>) -> Vec<RecentHistory> {
    let mut cutoffs: HashMap<String, i64> = HashMap::new();
    let mut histories = Vec::new();
    let mut grouper = HistoryGrouper::default();
    for row in rows {
        if let Some(cutoff) = row.cutoff_unix_s
            && !cutoffs.contains_key(&row.user_id)
        {
            cutoffs.insert(row.user_id.clone(), cutoff);
        }
        let Some(event) = event_of(row.track_id, row.event_code, row.unix_s, row.weight) else {
            continue;
        };
        if let Some(history) = grouper.push(&row.user_id, event) {
            histories.push(history);
        }
    }
    histories.extend(grouper.finish());
    histories
        .into_iter()
        .map(|history| RecentHistory {
            cutoff_unix_s: cutoffs.get(&history.user_id).copied(),
            history,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(user: &str, track: i64, unix_s: i64, cutoff: Option<i64>) -> RecentEventRow {
        RecentEventRow {
            user_id: user.to_owned(),
            track_id: track,
            event_code: 3,
            unix_s: Some(unix_s),
            weight: 0.3,
            cutoff_unix_s: cutoff,
        }
    }

    #[test]
    fn every_history_keeps_the_cut_of_its_own_user() {
        let histories = grouped(vec![
            row("1", 10, 5, None),
            row("1", 11, 6, None),
            row("2", 12, 7, Some(4)),
            row("2", 0, 8, Some(4)),
            row("3", 13, 9, Some(2)),
        ]);

        let shape: Vec<(&str, usize, Option<i64>)> = histories
            .iter()
            .map(|recent| {
                (
                    recent.history.user_id.as_str(),
                    recent.history.events.len(),
                    recent.cutoff_unix_s,
                )
            })
            .collect();
        assert_eq!(
            shape,
            [("1", 2, None), ("2", 1, Some(4)), ("3", 1, Some(2))]
        );
    }
}

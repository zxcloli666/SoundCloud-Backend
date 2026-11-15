use std::collections::{HashMap, HashSet};

use backend_contracts::pipeline::{MAX_MOMENT_HINTS, MomentHints};
use sqlx::PgPool;

use crate::queue::{JobError, JobResult};

const REPLAY_BUCKET_MS: i32 = 5_000;
const REPLAY_MIN_USERS: i64 = 3;
const REPLAY_MIN_BACKWARD_USERS: i64 = 2;
const CHORUS_GAP_MS: i32 = 8_000;
const MIN_LINE_CHARS: usize = 3;

struct HintRow {
    sc_track_id: String,
    synced_lrc: Option<String>,
    comment_ms: Vec<i32>,
    replay_ms: Vec<i32>,
}

pub(crate) async fn load(
    pool: &PgPool,
    sc_track_ids: &[String],
) -> JobResult<HashMap<String, MomentHints>> {
    if sc_track_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query_file_as!(
        HintRow,
        "queries/highlights/hints.sql",
        sc_track_ids,
        i64::from(MAX_MOMENT_HINTS),
        REPLAY_BUCKET_MS,
        REPLAY_MIN_USERS,
        REPLAY_MIN_BACKWARD_USERS
    )
    .fetch_all(pool)
    .await
    .map_err(JobError::retryable)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let hints = MomentHints {
                chorus_ms: row
                    .synced_lrc
                    .as_deref()
                    .map(chorus_starts)
                    .unwrap_or_default(),
                comment_ms: row.comment_ms,
                replay_ms: row.replay_ms,
            };
            (row.sc_track_id, hints)
        })
        .collect())
}

pub(crate) fn chorus_starts(synced_lrc: &str) -> Vec<i32> {
    let mut lines: Vec<(i32, String)> = synced_lrc.lines().flat_map(timed_line).collect();
    lines.sort_by_key(|(at, _)| *at);
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for (_, text) in &lines {
        *seen.entry(text.as_str()).or_default() += 1;
    }
    let repeated: HashSet<&str> = seen
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .map(|(text, _)| text)
        .collect();
    let mut starts = Vec::new();
    let mut previous: Option<(i32, bool)> = None;
    for (at, text) in &lines {
        let chorus = repeated.contains(text.as_str());
        let opens = match previous {
            Some((before, was_chorus)) => !was_chorus || at - before > CHORUS_GAP_MS,
            None => true,
        };
        if chorus && opens {
            starts.push(*at);
        }
        previous = Some((*at, chorus));
    }
    starts.truncate(usize::try_from(MAX_MOMENT_HINTS).unwrap_or(usize::MAX));
    starts
}

fn timed_line(line: &str) -> Vec<(i32, String)> {
    let mut rest = line.trim();
    let mut times = Vec::new();
    while let Some(tail) = rest.strip_prefix('[') {
        let Some((stamp, after)) = tail.split_once(']') else {
            break;
        };
        match stamp_ms(stamp) {
            Some(at) => times.push(at),
            None => return Vec::new(),
        }
        rest = after;
    }
    let text = normalized(rest);
    if text.chars().count() < MIN_LINE_CHARS {
        return Vec::new();
    }
    times.into_iter().map(|at| (at, text.clone())).collect()
}

fn stamp_ms(stamp: &str) -> Option<i32> {
    let (minutes, seconds) = stamp.split_once(':')?;
    let minutes: i32 = minutes.trim().parse().ok()?;
    let seconds: f64 = seconds.trim().parse().ok()?;
    if minutes < 0 || !(0.0..60.0).contains(&seconds) {
        return None;
    }
    let total = f64::from(minutes) * 60_000.0 + seconds * 1000.0;
    (total <= f64::from(i32::MAX)).then(|| total.round() as i32)
}

fn normalized(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repeated_block_gives_the_start_of_each_return() {
        let lrc = "[00:10.00]Walking down the road\n\
                   [00:15.00]Nobody else around\n\
                   [00:30.00]Oh, hold me now!\n\
                   [00:34.00]Never let me go\n\
                   [00:45.00]Second verse is here\n\
                   [01:20.50]oh hold me NOW\n\
                   [01:24.00]never let me go";

        assert_eq!(chorus_starts(lrc), [30_000, 80_500]);
    }

    #[test]
    fn a_line_with_several_stamps_counts_as_repeated() {
        let lrc = "[00:05.00]Intro words here\n[00:40.00][01:40.00]La la land again";

        assert_eq!(chorus_starts(lrc), [40_000, 100_000]);
    }

    #[test]
    fn no_repeats_no_tags_and_short_lines_give_nothing() {
        assert!(
            chorus_starts("[00:10.00]One\n[00:20.00]Two words\n[00:30.00]Three words").is_empty()
        );
        assert!(chorus_starts("[ar:Artist]\n[00:10.00]oh\n[00:20.00]oh").is_empty());
        assert!(chorus_starts("plain text\nplain text").is_empty());
        assert!(chorus_starts("").is_empty());
    }

    #[test]
    fn a_long_pause_inside_a_repeated_run_opens_a_new_start() {
        let lrc = "[00:30.00]Hold me now\n[00:34.00]Never let me go\n\
                   [01:30.00]Hold me now\n[01:34.00]Never let me go";

        assert_eq!(chorus_starts(lrc), [30_000, 90_000]);
    }
}

use chrono::{DateTime, Utc};

pub(crate) struct Round {
    pipeline: &'static str,
    groups: Vec<Group>,
}

struct Group {
    label: &'static str,
    picked: usize,
    waiting_seconds: i64,
}

impl Round {
    pub(crate) fn new(pipeline: &'static str) -> Self {
        Self {
            pipeline,
            groups: Vec::new(),
        }
    }

    pub(crate) fn record(
        &mut self,
        label: &'static str,
        picked: usize,
        next_waiting_since: Option<DateTime<Utc>>,
    ) {
        let waiting_seconds =
            next_waiting_since.map_or(0, |since| (Utc::now() - since).num_seconds().max(0));
        match self.groups.iter_mut().find(|group| group.label == label) {
            Some(group) => {
                group.picked += picked;
                group.waiting_seconds = waiting_seconds;
            }
            None => self.groups.push(Group {
                label,
                picked,
                waiting_seconds,
            }),
        }
    }

    pub(crate) fn report(self) {
        for group in &self.groups {
            crate::metrics::record_star_priority_round(
                self.pipeline,
                group.label,
                group.picked,
                group.waiting_seconds,
            );
        }
        if self
            .groups
            .iter()
            .all(|group| group.picked == 0 && group.waiting_seconds == 0)
        {
            return;
        }
        tracing::info!(
            pipeline = self.pipeline,
            tiers = %self.summary(),
            "work picked per tier and how long the next one in line has waited"
        );
    }

    fn summary(&self) -> String {
        self.groups
            .iter()
            .map(|group| {
                format!(
                    "{}={} next {}s",
                    group.label, group.picked, group.waiting_seconds
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub(crate) fn with_next(left: usize) -> i64 {
    i64::try_from(left).unwrap_or(i64::MAX).saturating_add(1)
}

pub(crate) fn split_next<T>(mut found: Vec<T>, left: usize) -> (Vec<T>, Option<T>) {
    let next = if found.len() > left {
        found.pop()
    } else {
        None
    };
    found.truncate(left);
    (found, next)
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    #[test]
    fn every_tier_shows_its_picks_and_the_wait_of_the_next_one_even_when_it_got_nothing() {
        let now = Utc::now();
        let mut round = Round::new("audio");
        round.record("star_liked", 2, None);
        round.record("liked", 0, Some(now - Duration::seconds(90)));
        round.record("rest", 1, Some(now - Duration::seconds(5)));

        let summary = round.summary();
        assert!(summary.starts_with("star_liked=2 next 0s, liked=0 next 9"));
        assert!(summary.contains("rest=1 next "));
    }

    #[test]
    fn repeated_batches_add_picks_and_keep_the_latest_wait() {
        let now = Utc::now();
        let mut round = Round::new("sync_queue");
        round.record("star", 1, Some(now - Duration::seconds(300)));
        round.record("star", 2, Some(now - Duration::seconds(5)));

        assert!(round.summary().starts_with("star=3 next 5s"));
        assert_eq!(round.groups.len(), 1);
    }

    #[test]
    fn the_row_past_the_room_is_the_next_one_in_line() {
        assert_eq!(split_next(vec![1, 2, 3], 2), (vec![1, 2], Some(3)));
        assert_eq!(split_next(vec![1, 2], 2), (vec![1, 2], None));
        assert_eq!(split_next(vec![7], 0), (Vec::<i32>::new(), Some(7)));
        assert_eq!(split_next(Vec::<i32>::new(), 0), (Vec::new(), None));
    }
}

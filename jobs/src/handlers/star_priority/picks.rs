use chrono::{DateTime, Utc};

pub(crate) struct Picks {
    pipeline: &'static str,
    groups: Vec<PickedGroup>,
}

struct PickedGroup {
    label: &'static str,
    picked: usize,
    oldest_wait_seconds: i64,
}

impl Picks {
    pub(crate) fn new(pipeline: &'static str) -> Self {
        Self {
            pipeline,
            groups: Vec::new(),
        }
    }

    pub(crate) fn record(
        &mut self,
        label: &'static str,
        waiting_since: impl IntoIterator<Item = DateTime<Utc>>,
    ) {
        let now = Utc::now();
        for since in waiting_since {
            let wait_seconds = (now - since).num_seconds();
            match self.groups.iter_mut().find(|group| group.label == label) {
                Some(group) => {
                    group.picked += 1;
                    group.oldest_wait_seconds = group.oldest_wait_seconds.max(wait_seconds);
                }
                None => self.groups.push(PickedGroup {
                    label,
                    picked: 1,
                    oldest_wait_seconds: wait_seconds,
                }),
            }
        }
    }

    pub(crate) fn report(self) {
        if self.groups.is_empty() {
            return;
        }
        for group in &self.groups {
            crate::metrics::record_star_priority_pick(
                self.pipeline,
                group.label,
                group.picked,
                group.oldest_wait_seconds,
            );
        }
        tracing::info!(
            pipeline = self.pipeline,
            picks = %self.summary(),
            "work picked per tier as count/oldest wait"
        );
    }

    fn summary(&self) -> String {
        self.groups
            .iter()
            .map(|group| {
                format!(
                    "{}={}/{}s",
                    group.label, group.picked, group.oldest_wait_seconds
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    #[test]
    fn each_tier_reports_its_count_and_oldest_wait() {
        let now = Utc::now();
        let mut picks = Picks::new("audio");
        picks.record(
            "star_liked",
            [now - Duration::seconds(30), now - Duration::seconds(90)],
        );
        picks.record("liked", []);
        picks.record("rest", [now - Duration::seconds(5)]);

        let summary = picks.summary();
        assert!(summary.starts_with("star_liked=2/9"));
        assert!(summary.contains(" rest=1/"));
        assert!(!summary.contains("liked=0"));
    }

    #[test]
    fn repeated_rounds_of_one_tier_add_up() {
        let now = Utc::now();
        let mut picks = Picks::new("sync_queue");
        picks.record("star", [now - Duration::seconds(5)]);
        picks.record("star", [now - Duration::seconds(300)]);

        assert!(picks.summary().starts_with("star=2/30"));
        assert_eq!(picks.groups.len(), 1);
    }
}

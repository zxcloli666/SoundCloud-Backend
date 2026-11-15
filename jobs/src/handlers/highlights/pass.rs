use std::time::{Duration, Instant};

use uuid::Uuid;

pub(super) struct Pass {
    after: Uuid,
    rest: Duration,
    resume_at: Option<Instant>,
}

impl Pass {
    pub(super) fn new(rest: Duration) -> Self {
        Self {
            after: Uuid::nil(),
            rest,
            resume_at: None,
        }
    }

    pub(super) fn after(&self) -> Uuid {
        self.after
    }

    pub(super) fn due(&self, now: Instant) -> bool {
        self.resume_at.is_none_or(|resume_at| now >= resume_at)
    }

    pub(super) fn advance(&mut self, next: Uuid, now: Instant) {
        self.after = next;
        self.resume_at = next.is_nil().then(|| now + self.rest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REST: Duration = Duration::from_secs(3_600);

    #[test]
    fn a_pass_walks_every_tick_until_the_table_ends() {
        let start = Instant::now();
        let mut pass = Pass::new(REST);
        let next = Uuid::now_v7();

        assert!(pass.due(start));
        pass.advance(next, start);

        assert!(pass.due(start));
        assert_eq!(pass.after(), next);
    }

    #[test]
    fn the_end_of_the_table_rests_before_the_next_pass_starts_over() {
        let start = Instant::now();
        let mut pass = Pass::new(REST);
        pass.advance(Uuid::now_v7(), start);

        pass.advance(Uuid::nil(), start);

        assert!(!pass.due(start + REST - Duration::from_secs(1)));
        assert!(pass.due(start + REST));
        assert_eq!(pass.after(), Uuid::nil());
    }
}

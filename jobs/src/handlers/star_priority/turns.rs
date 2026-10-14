use std::sync::atomic::{AtomicU64, Ordering};

use super::tier::Tier;

const REST_TURN_EVERY: u64 = 4;

#[derive(Default)]
pub(crate) struct Turns(AtomicU64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Turn {
    StarFirst,
    RestFirst,
}

impl Turns {
    pub(crate) fn next(&self) -> Turn {
        turn_at(self.0.fetch_add(1, Ordering::Relaxed))
    }
}

impl Turn {
    pub(crate) fn star_first(self) -> bool {
        self == Self::StarFirst
    }

    pub(crate) fn tiers(self) -> &'static [Tier] {
        match self {
            Self::StarFirst => &Tier::ORDER,
            Self::RestFirst => &[Tier::Rest],
        }
    }
}

fn turn_at(count: u64) -> Turn {
    if count.wrapping_add(1).is_multiple_of(REST_TURN_EVERY) {
        Turn::RestFirst
    } else {
        Turn::StarFirst
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fourth_turn_belongs_to_the_old_order() {
        let turns = Turns::default();
        let taken: Vec<Turn> = (0..8).map(|_| turns.next()).collect();
        assert_eq!(
            taken,
            [
                Turn::StarFirst,
                Turn::StarFirst,
                Turn::StarFirst,
                Turn::RestFirst,
                Turn::StarFirst,
                Turn::StarFirst,
                Turn::StarFirst,
                Turn::RestFirst,
            ]
        );
    }

    #[test]
    fn a_rest_turn_walks_only_the_old_order() {
        assert_eq!(Turn::RestFirst.tiers(), [Tier::Rest]);
        assert_eq!(Turn::StarFirst.tiers(), Tier::ORDER);
        assert!(Turn::StarFirst.star_first());
        assert!(!Turn::RestFirst.star_first());
    }
}

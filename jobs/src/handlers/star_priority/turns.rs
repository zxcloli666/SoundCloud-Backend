use std::sync::atomic::{AtomicU64, Ordering};

use super::tier::Tier;

const LEADS: [Tier; 8] = [
    Tier::StarLiked,
    Tier::StarLiked,
    Tier::Liked,
    Tier::Rest,
    Tier::StarLiked,
    Tier::StarOther,
    Tier::StarLiked,
    Tier::Rest,
];

#[derive(Default)]
pub(crate) struct Turns(AtomicU64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Turn {
    lead: Tier,
}

impl Turns {
    pub(crate) fn next(&self) -> Turn {
        let count = self.0.fetch_add(1, Ordering::Relaxed);
        let slot = usize::try_from(count % LEADS.len() as u64).unwrap_or(0);
        let lead = LEADS.get(slot).copied().unwrap_or(Tier::Rest);
        Turn { lead }
    }
}

impl Turn {
    #[cfg(test)]
    pub(crate) fn led_by(lead: Tier) -> Self {
        Self { lead }
    }

    pub(crate) fn star_first(self) -> bool {
        self.lead != Tier::Rest
    }

    pub(crate) fn tiers(self) -> impl Iterator<Item = Tier> {
        let lead = self.lead;
        let others = Tier::ORDER.into_iter().filter(move |tier| *tier != lead);
        std::iter::once(lead).chain(others)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leads(rounds: usize) -> Vec<Tier> {
        let turns = Turns::default();
        (0..rounds).map(|_| turns.next().lead).collect()
    }

    fn tiers_led_by(lead: Tier) -> Vec<Tier> {
        Turn::led_by(lead).tiers().collect()
    }

    #[test]
    fn every_lower_tier_leads_its_own_share_of_rounds() {
        let taken = leads(16);
        let share = |tier: Tier| taken.iter().filter(|lead| **lead == tier).count();
        assert_eq!(share(Tier::StarLiked), 8);
        assert_eq!(share(Tier::Liked), 2);
        assert_eq!(share(Tier::StarOther), 2);
        assert_eq!(share(Tier::Rest), 4);
    }

    #[test]
    fn the_rest_of_a_round_follows_the_tier_order() {
        assert_eq!(tiers_led_by(Tier::StarLiked), Tier::ORDER);
        assert_eq!(
            tiers_led_by(Tier::StarOther),
            [Tier::StarOther, Tier::StarLiked, Tier::Liked, Tier::Rest]
        );
        assert_eq!(
            tiers_led_by(Tier::Rest),
            [Tier::Rest, Tier::StarLiked, Tier::Liked, Tier::StarOther]
        );
    }

    #[test]
    fn only_a_rest_lead_puts_regular_users_first() {
        assert!(Turn::led_by(Tier::StarLiked).star_first());
        assert!(Turn::led_by(Tier::StarOther).star_first());
        assert!(!Turn::led_by(Tier::Rest).star_first());
    }
}

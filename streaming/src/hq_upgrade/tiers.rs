use tracing::info;

use crate::db::postgres::upgrade::UpgradeTier;

const LEADS: [UpgradeTier; 4] = [
    UpgradeTier::StarLiked,
    UpgradeTier::StarLiked,
    UpgradeTier::Liked,
    UpgradeTier::Rest,
];

const ORDER: [UpgradeTier; 3] = [
    UpgradeTier::StarLiked,
    UpgradeTier::Liked,
    UpgradeTier::Rest,
];

pub(super) fn for_turn(turn: u64) -> Vec<UpgradeTier> {
    let lead = LEADS[(turn % LEADS.len() as u64) as usize];
    std::iter::once(lead)
        .chain(ORDER.into_iter().filter(|tier| *tier != lead))
        .collect()
}

pub(super) struct Picked {
    pub(super) tier: UpgradeTier,
    pub(super) count: usize,
    pub(super) waiting_seconds: i64,
}

pub(super) fn report(stored: &'static str, round: &[Picked]) {
    for picked in round {
        crate::metrics::record_star_priority_round(
            stored,
            picked.tier.label(),
            picked.count,
            picked.waiting_seconds,
        );
    }
    if round
        .iter()
        .all(|picked| picked.count == 0 && picked.waiting_seconds == 0)
    {
        return;
    }
    info!(
        pipeline = "quality_upgrade",
        stored,
        tiers = %summary(round),
        "work picked per tier and how long the next one in line has waited"
    );
}

fn summary(round: &[Picked]) -> String {
    round
        .iter()
        .map(|picked| {
            format!(
                "{}={} next {}s",
                picked.tier.label(),
                picked.count,
                picked.waiting_seconds
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_likes_lead_half_the_turns_and_likes_and_the_old_order_a_quarter_each() {
        let leads: Vec<UpgradeTier> = (0..8).map(|turn| for_turn(turn)[0]).collect();
        let share = |tier: UpgradeTier| leads.iter().filter(|lead| **lead == tier).count();

        assert_eq!(share(UpgradeTier::StarLiked), 4);
        assert_eq!(share(UpgradeTier::Liked), 2);
        assert_eq!(share(UpgradeTier::Rest), 2);
    }

    #[test]
    fn the_rest_of_a_turn_follows_the_tier_order() {
        assert_eq!(for_turn(0), ORDER);
        assert_eq!(
            for_turn(2),
            [
                UpgradeTier::Liked,
                UpgradeTier::StarLiked,
                UpgradeTier::Rest
            ]
        );
        assert_eq!(
            for_turn(3),
            [
                UpgradeTier::Rest,
                UpgradeTier::StarLiked,
                UpgradeTier::Liked
            ]
        );
    }

    #[test]
    fn the_summary_names_every_tier_with_its_wait() {
        let round = [
            Picked {
                tier: UpgradeTier::StarLiked,
                count: 3,
                waiting_seconds: 0,
            },
            Picked {
                tier: UpgradeTier::Rest,
                count: 7,
                waiting_seconds: 42,
            },
        ];

        assert_eq!(summary(&round), "star_liked=3 next 0s, rest=7 next 42s");
    }
}

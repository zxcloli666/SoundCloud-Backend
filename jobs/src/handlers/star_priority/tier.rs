use std::collections::HashMap;

use catalog_ingest::TrackPriority;
use sqlx::PgExecutor;

pub(crate) const CORE_FAST_TRACK_PRIORITY: i16 = 6;
pub(crate) const INDEX_TRACK_PRIORITY: i16 = 5;
pub(crate) const LYRICS_EMBED_PRIORITY: i16 = 0;
pub(crate) const LIKED_PRIORITY: i16 = TrackPriority::Like as i16;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Tier {
    StarLiked,
    Liked,
    StarOther,
    Rest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Pass {
    Star(i16),
    Liked,
    Rest,
}

impl Tier {
    pub(crate) const ORDER: [Self; 4] = [Self::StarLiked, Self::Liked, Self::StarOther, Self::Rest];

    pub(crate) fn pass(self) -> Pass {
        match self {
            Self::StarLiked => Pass::Star(1),
            Self::Liked => Pass::Liked,
            Self::StarOther => Pass::Star(3),
            Self::Rest => Pass::Rest,
        }
    }

    pub(crate) fn job_priority(self, base: i16) -> i16 {
        base + match self {
            Self::StarLiked => 3,
            Self::Liked => 2,
            Self::StarOther => 1,
            Self::Rest => 0,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::StarLiked => "star_liked",
            Self::Liked => "liked",
            Self::StarOther => "star_other",
            Self::Rest => "rest",
        }
    }

    fn from_rank(rank: i16) -> Self {
        match rank {
            1 => Self::StarLiked,
            2 => Self::Liked,
            3 => Self::StarOther,
            _ => Self::Rest,
        }
    }
}

pub(crate) async fn of_tracks<'e>(
    executor: impl PgExecutor<'e>,
    sc_track_ids: &[String],
) -> Result<HashMap<String, Tier>, sqlx::Error> {
    if sc_track_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query_file!(
        "queries/star_priority/track_tiers.sql",
        sc_track_ids,
        LIKED_PRIORITY
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.sc_track_id, Tier::from_rank(row.rank)))
        .collect())
}

pub(crate) async fn of_track<'e>(
    executor: impl PgExecutor<'e>,
    sc_track_id: &str,
) -> Result<Tier, sqlx::Error> {
    let tiers = of_tracks(executor, &[sc_track_id.to_owned()]).await?;
    Ok(tiers.get(sc_track_id).copied().unwrap_or(Tier::Rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_likes_come_first_and_everything_else_keeps_its_old_order_last() {
        assert_eq!(
            Tier::ORDER.map(Tier::pass),
            [Pass::Star(1), Pass::Liked, Pass::Star(3), Pass::Rest]
        );
    }

    #[test]
    fn job_priorities_fall_with_the_tier() {
        let priorities = Tier::ORDER.map(|tier| tier.job_priority(CORE_FAST_TRACK_PRIORITY));
        assert_eq!(priorities, [9, 8, 7, 6]);
    }

    #[test]
    fn a_like_is_the_catalog_like_priority() {
        assert_eq!(LIKED_PRIORITY, 1);
    }

    #[test]
    fn unknown_ranks_are_the_rest() {
        assert_eq!(Tier::from_rank(1), Tier::StarLiked);
        assert_eq!(Tier::from_rank(4), Tier::Rest);
        assert_eq!(Tier::from_rank(9), Tier::Rest);
    }
}

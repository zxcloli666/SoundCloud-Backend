use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum EventKind {
    Like,
    LikeImport,
    PlaylistAdd,
    FullPlay,
    Skip,
    Dislike,
}

impl EventKind {
    pub(crate) const ALL: [Self; 6] = [
        Self::Like,
        Self::LikeImport,
        Self::PlaylistAdd,
        Self::FullPlay,
        Self::Skip,
        Self::Dislike,
    ];

    pub(crate) fn code(self) -> u8 {
        match self {
            Self::Like => 0,
            Self::LikeImport => 1,
            Self::PlaylistAdd => 2,
            Self::FullPlay => 3,
            Self::Skip => 4,
            Self::Dislike => 5,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Like => "like",
            Self::LikeImport => "like_import",
            Self::PlaylistAdd => "playlist_add",
            Self::FullPlay => "full_play",
            Self::Skip => "skip",
            Self::Dislike => "dislike",
        }
    }

    pub(crate) fn from_code(code: i16) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| i16::from(kind.code()) == code)
    }

    pub(crate) fn from_label(label: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.name() == label || kind.code().to_string() == label)
    }

    fn declares_a_positive(self) -> bool {
        matches!(self, Self::Like | Self::LikeImport | Self::PlaylistAdd)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TasteEvent {
    pub track: u64,
    pub kind: EventKind,
    pub unix_s: Option<i64>,
    pub weight: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UserHistory {
    pub user_id: String,
    pub events: Vec<TasteEvent>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Positives {
    pub total: usize,
    pub timed: usize,
}

impl UserHistory {
    pub(crate) fn positives(&self) -> Positives {
        let disliked: HashSet<u64> = self
            .events
            .iter()
            .filter(|event| event.kind == EventKind::Dislike)
            .map(|event| event.track)
            .collect();
        let mut full_plays: HashMap<u64, usize> = HashMap::new();
        let mut declared: HashSet<u64> = HashSet::new();
        let mut declared_in_time: HashSet<u64> = HashSet::new();
        for event in &self.events {
            if event.kind == EventKind::FullPlay {
                *full_plays.entry(event.track).or_default() += 1;
            } else if event.kind.declares_a_positive() {
                declared.insert(event.track);
                if event.unix_s.is_some() {
                    declared_in_time.insert(event.track);
                }
            }
        }
        let replayed: HashSet<u64> = full_plays
            .into_iter()
            .filter(|(_, plays)| *plays >= 2)
            .map(|(track, _)| track)
            .collect();
        let liked = |track: &&u64| !disliked.contains(*track);
        Positives {
            total: declared.union(&replayed).filter(liked).count(),
            timed: declared_in_time.union(&replayed).filter(liked).count(),
        }
    }
}

#[derive(Default)]
pub(crate) struct HistoryGrouper {
    current: Option<UserHistory>,
}

impl HistoryGrouper {
    pub(crate) fn push(&mut self, user_id: &str, event: TasteEvent) -> Option<UserHistory> {
        if let Some(current) = self.current.as_mut()
            && current.user_id == user_id
        {
            current.events.push(event);
            return None;
        }
        self.current.replace(UserHistory {
            user_id: user_id.to_owned(),
            events: vec![event],
        })
    }

    pub(crate) fn finish(self) -> Option<UserHistory> {
        self.current
    }
}

pub(crate) fn event_of(
    track_id: i64,
    event_code: i16,
    unix_s: Option<i64>,
    weight: f64,
) -> Option<TasteEvent> {
    Some(TasteEvent {
        track: u64::try_from(track_id).ok().filter(|track| *track > 0)?,
        kind: EventKind::from_code(event_code)?,
        unix_s,
        weight,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(track: u64, kind: EventKind, unix_s: Option<i64>) -> TasteEvent {
        TasteEvent {
            track,
            kind,
            unix_s,
            weight: 1.0,
        }
    }

    fn history(events: Vec<TasteEvent>) -> UserHistory {
        UserHistory {
            user_id: "7".to_owned(),
            events,
        }
    }

    #[test]
    fn a_single_full_play_is_history_and_a_second_one_makes_a_positive() {
        let once = history(vec![event(1, EventKind::FullPlay, Some(10))]);
        let twice = history(vec![
            event(1, EventKind::FullPlay, Some(10)),
            event(1, EventKind::FullPlay, Some(20)),
        ]);

        assert_eq!(once.positives(), Positives::default());
        assert_eq!(twice.positives(), Positives { total: 1, timed: 1 });
    }

    #[test]
    fn an_imported_like_counts_as_taste_but_never_as_a_timed_positive() {
        let user = history(vec![
            event(1, EventKind::LikeImport, None),
            event(2, EventKind::LikeImport, None),
            event(2, EventKind::PlaylistAdd, Some(30)),
        ]);

        assert_eq!(user.positives(), Positives { total: 2, timed: 1 });
    }

    const SHARED_FIXTURE: &str =
        include_str!("../../../../backend-contracts/fixtures/taste/positives.jsonl");

    fn fixture_histories() -> (usize, Vec<UserHistory>) {
        let mut lines = SHARED_FIXTURE.lines();
        let header: serde_json::Value = lines
            .next()
            .and_then(|line| serde_json::from_str(line).ok())
            .unwrap_or_default();
        let expected = header["users_with_5_timed_positives"]
            .as_u64()
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or(usize::MAX);
        let histories = lines
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .map(|user| UserHistory {
                user_id: user["u"].as_str().unwrap_or_default().to_owned(),
                events: user["e"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|event| {
                        event_of(
                            event[0].as_i64()?,
                            i16::try_from(event[1].as_i64()?).ok()?,
                            event[2].as_i64(),
                            event[3].as_f64()?,
                        )
                    })
                    .collect(),
            })
            .collect();
        (expected, histories)
    }

    #[test]
    fn the_export_gate_counts_timed_positives_like_the_worker_split() {
        let (expected, histories) = fixture_histories();

        let counted = histories
            .iter()
            .filter(|history| history.positives().timed >= 5)
            .count();

        assert_eq!(histories.len(), 7);
        assert_eq!(counted, expected);
        assert_eq!(counted, 4);
    }

    #[test]
    fn a_disliked_track_is_no_positive_whatever_else_happened_to_it() {
        let user = history(vec![
            event(1, EventKind::Like, Some(10)),
            event(1, EventKind::FullPlay, Some(11)),
            event(1, EventKind::FullPlay, Some(12)),
            event(1, EventKind::Dislike, Some(13)),
            event(2, EventKind::Skip, Some(14)),
        ]);

        assert_eq!(user.positives(), Positives::default());
    }

    #[test]
    fn rows_are_grouped_into_one_history_per_user() {
        let mut grouper = HistoryGrouper::default();

        assert_eq!(grouper.push("1", event(10, EventKind::Like, Some(1))), None);
        assert_eq!(grouper.push("1", event(11, EventKind::Like, Some(2))), None);
        let first = grouper.push("2", event(12, EventKind::Like, Some(3)));

        assert_eq!(
            first.map(|user| (user.user_id, user.events.len())),
            Some(("1".to_owned(), 2))
        );
        assert_eq!(
            grouper
                .finish()
                .map(|user| (user.user_id, user.events.len())),
            Some(("2".to_owned(), 1))
        );
    }

    #[test]
    fn a_row_outside_the_known_kinds_or_with_a_bad_track_is_dropped() {
        assert!(event_of(0, 0, None, 1.0).is_none());
        assert!(event_of(-4, 0, None, 1.0).is_none());
        assert!(event_of(4, 9, None, 1.0).is_none());
        assert_eq!(
            event_of(4, 5, Some(9), -1.0),
            Some(TasteEvent {
                track: 4,
                kind: EventKind::Dislike,
                unix_s: Some(9),
                weight: -1.0
            })
        );
    }

    #[test]
    fn kinds_are_named_like_the_dataset_header_and_its_codes() {
        for kind in EventKind::ALL {
            assert_eq!(EventKind::from_label(kind.name()), Some(kind));
            assert_eq!(EventKind::from_label(&kind.code().to_string()), Some(kind));
            assert_eq!(EventKind::from_code(i16::from(kind.code())), Some(kind));
        }
    }
}

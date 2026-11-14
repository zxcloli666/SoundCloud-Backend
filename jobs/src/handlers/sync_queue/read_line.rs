use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use uuid::Uuid;

const LONGEST_WAIT: Duration = Duration::from_secs(6 * 60 * 60);
const SHORTEST_WAIT: Duration = Duration::from_secs(1);
const EARLY: Duration = Duration::from_secs(2);
const LATE: Duration = Duration::from_secs(10 * 60);
const REMEMBERED: Duration = Duration::from_secs(15 * 60);
const AT_THE_DOOR: usize = 4;

#[derive(Clone, Copy, Debug)]
pub struct Waiter {
    pub job: Uuid,
    pub priority: i16,
}

type Place = (Reverse<i16>, Instant, Uuid);

struct Served {
    since: Instant,
    at: Instant,
}

struct Ahead {
    waiting: usize,
    due: usize,
}

#[derive(Default)]
pub(super) struct Line {
    due: BTreeMap<Place, Instant>,
    places: HashMap<Uuid, Place>,
    served: HashMap<Uuid, Served>,
}

impl Line {
    pub(super) fn turn(
        &mut self,
        waiter: Waiter,
        opens_in: Duration,
        gap: Duration,
        now: Instant,
    ) -> Option<Duration> {
        let place = self.place(waiter, now);
        let ahead = self.ahead(&place, now);
        if opens_in.is_zero() && ahead.due < AT_THE_DOOR {
            self.serve(place, now);
            return None;
        }
        let wait = wait(opens_in, gap, ahead.waiting, waiter.job);
        self.due.insert(place, now + wait);
        Some(wait)
    }

    fn place(&mut self, waiter: Waiter, now: Instant) -> Place {
        let since = match self.places.get(&waiter.job) {
            Some(place) if place.0.0 == waiter.priority => return *place,
            Some(place) => {
                let since = place.1;
                self.due.remove(place);
                since
            }
            None => self
                .served
                .get(&waiter.job)
                .filter(|served| now.saturating_duration_since(served.at) < REMEMBERED)
                .map_or(now, |served| served.since),
        };
        let place = (Reverse(waiter.priority), since, waiter.job);
        self.places.insert(waiter.job, place);
        place
    }

    fn ahead(&mut self, place: &Place, now: Instant) -> Ahead {
        let mut ahead = Ahead { waiting: 0, due: 0 };
        let mut gone = Vec::new();
        for (other, due) in self.due.range(..*place) {
            if now.saturating_duration_since(*due) > LATE {
                gone.push(*other);
                continue;
            }
            ahead.waiting += 1;
            if *due <= now + EARLY {
                ahead.due += 1;
            }
        }
        for other in gone {
            self.due.remove(&other);
            self.places.remove(&other.2);
        }
        ahead
    }

    fn serve(&mut self, place: Place, now: Instant) {
        self.due.remove(&place);
        self.places.remove(&place.2);
        self.served
            .retain(|_, served| now.saturating_duration_since(served.at) < REMEMBERED);
        self.served.insert(
            place.2,
            Served {
                since: place.1,
                at: now,
            },
        );
    }
}

fn wait(opens_in: Duration, gap: Duration, ahead: usize, job: Uuid) -> Duration {
    let turns = u32::try_from(ahead).unwrap_or(u32::MAX);
    let turn = opens_in.saturating_add(gap.saturating_mul(turns));
    if turn < LONGEST_WAIT {
        return turn.max(SHORTEST_WAIT);
    }
    let share = f64::from(job.as_bytes()[15]) / 255.0;
    LONGEST_WAIT.mul_f64(0.5 + share / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAP: Duration = Duration::from_secs(10);
    const PAUSE: Duration = Duration::from_secs(300);

    fn waiter(priority: i16) -> Waiter {
        Waiter {
            job: Uuid::now_v7(),
            priority,
        }
    }

    #[test]
    fn waiters_are_told_to_return_one_gap_apart_in_the_order_they_came() {
        let mut line = Line::default();
        let now = Instant::now();
        let waiters: Vec<_> = (0..5).map(|_| waiter(20)).collect();

        let waits: Vec<_> = waiters
            .iter()
            .enumerate()
            .map(|(index, waiter)| {
                line.turn(
                    *waiter,
                    PAUSE,
                    GAP,
                    now + Duration::from_millis(index as u64),
                )
            })
            .collect();

        assert_eq!(
            waits,
            (0..5)
                .map(|ahead| Some(PAUSE + GAP * ahead))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_higher_priority_waiter_stands_ahead_of_everyone_who_came_before() {
        let mut line = Line::default();
        let now = Instant::now();
        for _ in 0..50 {
            line.turn(waiter(20), PAUSE, GAP, now);
        }

        let boosted = waiter(25);
        assert_eq!(line.turn(boosted, PAUSE, GAP, now), Some(PAUSE));

        let ordinary = waiter(20);
        assert_eq!(
            line.turn(ordinary, PAUSE, GAP, now + Duration::from_millis(1)),
            Some(PAUSE + GAP * 51)
        );
    }

    #[test]
    fn an_open_application_serves_the_front_and_keeps_the_rest_waiting() {
        let mut line = Line::default();
        let now = Instant::now();
        let waiters: Vec<_> = (0..10).map(|_| waiter(20)).collect();
        for (index, waiter) in waiters.iter().enumerate() {
            line.turn(
                *waiter,
                PAUSE,
                GAP,
                now + Duration::from_millis(index as u64),
            );
        }
        let opened = now + PAUSE + GAP * 4;

        assert_eq!(
            line.turn(waiters[9], Duration::ZERO, GAP, opened),
            Some(GAP * 9),
            "five waiters ahead are already due, the tenth must not take their read"
        );
        assert_eq!(line.turn(waiters[0], Duration::ZERO, GAP, opened), None);
        assert_eq!(line.turn(waiters[3], Duration::ZERO, GAP, opened), None);
    }

    #[test]
    fn a_waiter_whose_turn_is_far_takes_a_free_read_while_the_front_sleeps() {
        let mut line = Line::default();
        let now = Instant::now();
        let waiters: Vec<_> = (0..40).map(|_| waiter(20)).collect();
        for (index, waiter) in waiters.iter().enumerate() {
            line.turn(
                *waiter,
                PAUSE,
                PAUSE,
                now + Duration::from_millis(index as u64),
            );
        }

        let early = now + PAUSE / 2;

        assert_eq!(line.turn(waiters[30], Duration::ZERO, GAP, early), None);
    }

    #[test]
    fn a_served_job_keeps_its_place_for_the_next_page() {
        let mut line = Line::default();
        let now = Instant::now();
        let walker = waiter(20);
        line.turn(walker, PAUSE, GAP, now);
        for _ in 0..30 {
            line.turn(waiter(20), PAUSE, GAP, now + Duration::from_secs(1));
        }
        let opened = now + PAUSE;
        assert_eq!(line.turn(walker, Duration::ZERO, GAP, opened), None);

        let next_page = opened + Duration::from_secs(3);

        assert_eq!(line.turn(walker, GAP, GAP, next_page), Some(GAP));
    }

    #[test]
    fn a_waiter_that_never_returned_stops_holding_the_door() {
        let mut line = Line::default();
        let now = Instant::now();
        for _ in 0..AT_THE_DOOR {
            line.turn(waiter(25), PAUSE, GAP, now);
        }
        let patient = waiter(20);
        line.turn(patient, PAUSE, GAP, now);
        let opened = now + PAUSE + GAP * 4;

        assert!(line.turn(patient, Duration::ZERO, GAP, opened).is_some());
        assert_eq!(
            line.turn(patient, Duration::ZERO, GAP, opened + LATE + GAP),
            None
        );
        assert_eq!(line.due.len(), 0);
        assert_eq!(line.places.len(), 0);
    }

    #[test]
    fn a_turn_beyond_six_hours_is_spread_over_the_last_three() {
        let mut line = Line::default();
        let now = Instant::now();
        let waits: Vec<_> = (0..400)
            .filter_map(|index| {
                line.turn(waiter(20), PAUSE, PAUSE, now + Duration::from_millis(index))
            })
            .collect();

        let far = &waits[100..];
        assert!(
            far.iter()
                .all(|wait| *wait >= LONGEST_WAIT / 2 && *wait <= LONGEST_WAIT)
        );
        let distinct: std::collections::HashSet<_> = far.iter().collect();
        assert!(distinct.len() > 50, "{}", distinct.len());
    }

    #[test]
    fn a_raised_priority_moves_the_waiter_forward_without_losing_its_age() {
        let mut line = Line::default();
        let now = Instant::now();
        let mut raised = waiter(20);
        line.turn(raised, PAUSE, GAP, now);
        for _ in 0..20 {
            line.turn(waiter(25), PAUSE, GAP, now + Duration::from_secs(1));
        }
        let late_boosted = waiter(25);
        line.turn(late_boosted, PAUSE, GAP, now + Duration::from_secs(2));

        raised.priority = 25;

        assert_eq!(
            line.turn(raised, PAUSE, GAP, now + Duration::from_secs(3)),
            Some(PAUSE)
        );
        assert_eq!(line.due.len(), 22);
    }
}

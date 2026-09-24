use std::time::Duration;

use uuid::Uuid;

const BASE_SECONDS: u64 = 2;
const MAX_SECONDS: u64 = 15 * 60;
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

const IDLE_MAX: Duration = Duration::from_secs(2);

pub fn idle_wait(empty_polls: u32, poll_interval: Duration) -> Duration {
    let doublings = empty_polls.saturating_sub(1).min(16);
    poll_interval
        .saturating_mul(2_u32.saturating_pow(doublings))
        .min(IDLE_MAX)
        .max(poll_interval)
}

pub fn retry_delay(job_id: Uuid, attempt: i32) -> Duration {
    let exponent = u32::try_from(attempt.saturating_sub(1).max(0)).map_or(0, |value| value.min(8));
    let base = BASE_SECONDS.saturating_mul(2_u64.saturating_pow(exponent));
    let jitter = retry_hash(job_id, attempt) % (base / 4 + 1);
    Duration::from_secs(base.saturating_add(jitter).min(MAX_SECONDS))
}

fn retry_hash(job_id: Uuid, attempt: i32) -> u64 {
    let attempt = attempt.to_le_bytes();
    job_id
        .as_bytes()
        .iter()
        .chain(attempt.iter())
        .fold(FNV_OFFSET, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_empty_poll_waits_exactly_as_before() {
        let poll = Duration::from_millis(250);
        assert_eq!(idle_wait(0, poll), poll);
        assert_eq!(
            idle_wait(1, poll),
            poll,
            "one empty answer is ordinary; the queue must not become slow to react after it"
        );
    }

    #[test]
    fn an_idle_worker_stops_asking_four_times_a_second() {
        let poll = Duration::from_millis(250);
        assert!(idle_wait(2, poll) > poll);
        assert!(idle_wait(3, poll) > idle_wait(2, poll));
        assert!(
            idle_wait(8, poll) >= Duration::from_secs(2),
            "an idle worker reads its whole lane on every ask, so the interval has to grow"
        );
    }

    #[test]
    fn the_wait_never_grows_past_two_seconds() {
        for poll in [Duration::from_millis(10), Duration::from_millis(250)] {
            assert_eq!(
                idle_wait(u32::MAX, poll),
                IDLE_MAX,
                "a queue nobody is using must still pick up new work within two seconds"
            );
        }
    }

    #[test]
    fn a_poll_interval_larger_than_the_ceiling_is_respected_as_asked() {
        let poll = Duration::from_secs(5);
        assert_eq!(idle_wait(0, poll), poll);
        assert_eq!(idle_wait(9, poll), poll);
    }

    #[test]
    fn retries_grow_and_stay_bounded() {
        let id = Uuid::nil();
        assert!(retry_delay(id, 2) > retry_delay(id, 1));
        assert!(retry_delay(id, i32::MAX) <= Duration::from_secs(MAX_SECONDS));
    }

    #[test]
    fn nearby_uuid_v7_jobs_receive_different_jitter() {
        let first = Uuid::from_bytes([1, 143, 0, 0, 0, 0, 112, 0, 128, 0, 0, 0, 0, 0, 0, 1]);
        let second = Uuid::from_bytes([1, 143, 0, 0, 0, 0, 112, 0, 128, 0, 0, 0, 0, 0, 0, 2]);

        assert_ne!(retry_delay(first, 6), retry_delay(second, 6));
    }
}

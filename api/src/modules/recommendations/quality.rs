pub const MIN_DURATION_MS: i32 = 30_000;
pub const MAX_DURATION_MS: i32 = 30 * 60_000;
pub const MIN_PLAYS_DEFAULT: i64 = 50;

pub struct QualityCheck<'a> {
    pub duration_ms: i32,
    pub title: &'a str,
    pub plays: i64,
}

pub fn passes(check: QualityCheck<'_>, min_plays: i64) -> bool {
    if check.plays < min_plays {
        return false;
    }
    if check.duration_ms < MIN_DURATION_MS || check.duration_ms > MAX_DURATION_MS {
        return false;
    }
    let lower = check.title.to_lowercase();
    if lower.contains("preview") || lower.contains("teaser") {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(duration_ms: i32, title: &str, plays: i64) -> QualityCheck<'_> {
        QualityCheck {
            duration_ms,
            title,
            plays,
        }
    }

    #[test]
    fn an_ordinary_track_passes() {
        assert!(passes(
            track(180_000, "Real song", 1_000),
            MIN_PLAYS_DEFAULT
        ));
    }

    #[test]
    fn the_duration_window_is_closed_on_both_ends() {
        assert!(!passes(
            track(MIN_DURATION_MS - 1, "Too short", 1_000),
            MIN_PLAYS_DEFAULT
        ));
        assert!(passes(
            track(MIN_DURATION_MS, "Exactly the floor", 1_000),
            MIN_PLAYS_DEFAULT
        ));
        assert!(passes(
            track(MAX_DURATION_MS, "Exactly the ceiling", 1_000),
            MIN_PLAYS_DEFAULT
        ));
        assert!(!passes(
            track(MAX_DURATION_MS + 1, "A whole DJ set", 1_000),
            MIN_PLAYS_DEFAULT
        ));
    }

    #[test]
    fn a_preview_is_rejected_whatever_the_case() {
        for title in ["preview", "PREVIEW", "Album PreVieW", "TEASER cut"] {
            assert!(
                !passes(track(180_000, title, 10_000), MIN_PLAYS_DEFAULT),
                "{title} must not reach the wave"
            );
        }
    }

    #[test]
    fn the_play_floor_is_the_argument_and_not_the_constant() {
        assert!(!passes(track(180_000, "New upload", 0), MIN_PLAYS_DEFAULT));
        assert!(
            passes(track(180_000, "New upload", 0), 0),
            "a caller that wants cold tracks must be able to ask for them"
        );
    }

    #[test]
    fn the_floor_admits_a_track_that_is_exactly_on_it() {
        assert!(passes(
            track(180_000, "Right on the floor", MIN_PLAYS_DEFAULT),
            MIN_PLAYS_DEFAULT
        ));
    }
}

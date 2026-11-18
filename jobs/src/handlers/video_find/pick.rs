use catalog_match::title_score;

use super::ytdlp::Found;

const NEGATIVE: [&str; 17] = [
    "live",
    "cover",
    "remix",
    "nightcore",
    "sped up",
    "slowed",
    "8d",
    "karaoke",
    "reverb",
    "instrumental",
    "mashup",
    "reaction",
    "tutorial",
    "bass boosted",
    "loop",
    "1 hour",
    "extended",
];
const MIN_S: f64 = 20.0;
const MAX_S: f64 = 600.0;
const RATIO_LOW: f64 = 0.7;
const RATIO_HIGH: f64 = 1.8;
const SLACK_S: f64 = 30.0;

pub(super) fn pick(
    found: Vec<Found>,
    artist: &str,
    title: &str,
    duration_s: Option<f64>,
    take: usize,
) -> Vec<Found> {
    let ours = words(&format!("{artist} {title}"));
    let mut allowed: Vec<(f32, Found)> = found
        .into_iter()
        .filter(|item| plausible(item.duration_s, duration_s) && !unwanted(&item.title, &ours))
        .map(|item| (title_score(title, &item.title, None), item))
        .collect();
    allowed.sort_by(|left, right| right.0.total_cmp(&left.0));
    allowed
        .into_iter()
        .map(|(_, item)| item)
        .take(take)
        .collect()
}

fn plausible(candidate_s: Option<f64>, track_s: Option<f64>) -> bool {
    let Some(candidate_s) = candidate_s else {
        return false;
    };
    if !(MIN_S..=MAX_S).contains(&candidate_s) {
        return false;
    }
    let Some(track_s) = track_s.filter(|track| *track > 0.0) else {
        return true;
    };
    let ratio = candidate_s / track_s;
    (candidate_s - track_s).abs() <= SLACK_S || (RATIO_LOW..=RATIO_HIGH).contains(&ratio)
}

fn unwanted(candidate_title: &str, ours: &str) -> bool {
    let theirs = format!(" {} ", words(candidate_title));
    let mine = format!(" {ours} ");
    NEGATIVE.iter().any(|word| {
        let padded = format!(" {word} ");
        theirs.contains(&padded) && !mine.contains(&padded)
    })
}

fn words(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(id: &str, title: &str, duration_s: Option<f64>) -> Found {
        Found {
            id: id.to_owned(),
            title: title.to_owned(),
            duration_s,
        }
    }

    #[test]
    fn covers_remixes_and_wrong_lengths_are_dropped_and_the_closest_title_leads() {
        let picked = pick(
            vec![
                found("a", "Artist - Song (Official Video)", Some(200.0)),
                found("b", "Artist - Song (Nightcore)", Some(200.0)),
                found("c", "Artist - Song", Some(3000.0)),
                found("d", "Artist - Song", None),
                found("e", "Artist - Song live", Some(200.0)),
                found("f", "Artist Song", Some(205.0)),
            ],
            "Artist",
            "Song",
            Some(200.0),
            3,
        );

        let ids: Vec<_> = picked.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"a") && ids.contains(&"f"));
    }

    #[test]
    fn a_word_the_track_itself_has_is_not_a_negative() {
        let picked = pick(
            vec![found("a", "Band - Live Forever", Some(200.0))],
            "Band",
            "Live Forever",
            Some(200.0),
            3,
        );

        assert_eq!(picked.len(), 1);
    }

    #[test]
    fn an_unknown_track_length_keeps_only_the_hard_bounds() {
        let picked = pick(
            vec![found("a", "x", Some(10.0)), found("b", "y", Some(300.0))],
            "x",
            "y",
            None,
            3,
        );

        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].id, "b");
    }
}

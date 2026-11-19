use crate::datadome;
use crate::journal::{Outcome, Row};

const ACTIONS: [&str; 5] = ["like", "repost", "comment_like", "follow", "playlist"];

pub fn lines(rows: &[Row]) -> Vec<String> {
    ACTIONS
        .iter()
        .filter_map(|action| verdict(rows, action).map(|text| format!("{action}: {text}")))
        .collect()
}

fn verdict(rows: &[Row], action: &str) -> Option<String> {
    let prefix = format!("{action}.");
    let own: Vec<&Row> = rows
        .iter()
        .enumerate()
        .filter(|(index, row)| {
            row.step.starts_with(&prefix)
                && row.outcome != Outcome::Skipped
                && !is_superseded(rows, *index)
        })
        .map(|(_, row)| row)
        .collect();
    if own.is_empty() {
        return None;
    }
    if own
        .iter()
        .any(|row| row.notes.iter().any(|note| note == datadome::NOTE))
    {
        return Some("datadome".to_owned());
    }
    let failed = own.iter().find(|row| row.outcome == Outcome::Failed);
    if let Some(status) = failed.and_then(|row| row.status) {
        return Some(format!("refused {status}"));
    }
    let unclear = failed.is_some()
        || own
            .iter()
            .any(|row| row.notes.iter().any(|note| note.starts_with("unclear")));
    if unclear {
        return Some("unclear".to_owned());
    }
    Some(match winners(&own, &prefix) {
        Some(won) => format!("ok ({won})"),
        None => "ok".to_owned(),
    })
}

fn winners(own: &[&Row], prefix: &str) -> Option<String> {
    let won: Vec<String> = own
        .iter()
        .filter(|row| row.outcome == Outcome::Passed && row.step.contains('#'))
        .filter_map(|row| row.step.strip_prefix(prefix))
        .map(|step| step.replace('#', ": "))
        .collect();
    (!won.is_empty()).then(|| won.join(", "))
}

pub fn is_superseded(rows: &[Row], index: usize) -> bool {
    let row = &rows[index];
    let Some((stem, _)) = row.step.split_once('#') else {
        return false;
    };
    row.outcome == Outcome::Failed
        && rows[index + 1..].iter().any(|later| {
            later.outcome == Outcome::Passed
                && later.step.split_once('#').map(|(later_stem, _)| later_stem) == Some(stem)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(step: &str, status: Option<u16>, outcome: Outcome, note: &str) -> Row {
        Row {
            step: step.to_owned(),
            status,
            plan: None,
            outcome,
            notes: vec![note.to_owned()],
        }
    }

    #[test]
    fn each_action_gets_one_verdict() {
        let rows = [
            row("like.create", Some(200), Outcome::Passed, "liked"),
            row("like.read", None, Outcome::Skipped, "no read"),
            row("repost.add", Some(403), Outcome::Failed, "datadome"),
            row("follow.create", Some(404), Outcome::Failed, "answer"),
            row("playlist.create", Some(200), Outcome::Passed, "unclear: x"),
        ];

        assert_eq!(
            lines(&rows),
            vec![
                "like: ok",
                "repost: datadome",
                "follow: refused 404",
                "playlist: unclear"
            ]
        );
    }

    #[test]
    fn a_refused_shape_is_dropped_when_a_later_one_wins() {
        let rows = [
            row("playlist.create#flat", Some(400), Outcome::Failed, "answer"),
            row(
                "playlist.create#wrapped",
                Some(200),
                Outcome::Passed,
                "created",
            ),
            row("repost.add#post", Some(405), Outcome::Failed, "answer"),
            row("repost.add#put", Some(200), Outcome::Passed, "reposted"),
        ];

        assert!(is_superseded(&rows, 0));
        assert!(!is_superseded(&rows, 1));
        assert_eq!(
            lines(&rows),
            vec!["repost: ok (add: put)", "playlist: ok (create: wrapped)"]
        );
    }

    #[test]
    fn every_shape_refused_stays_refused() {
        let rows = [
            row("playlist.create#flat", Some(400), Outcome::Failed, "answer"),
            row(
                "playlist.create#wrapped",
                Some(422),
                Outcome::Failed,
                "answer",
            ),
        ];

        assert!(!is_superseded(&rows, 0));
        assert_eq!(lines(&rows), vec!["playlist: refused 400"]);
    }
}

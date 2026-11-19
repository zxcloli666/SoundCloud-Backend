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
        .filter(|row| row.step.starts_with(&prefix) && row.outcome != Outcome::Skipped)
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
    Some(if unclear { "unclear" } else { "ok" }.to_owned())
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
}

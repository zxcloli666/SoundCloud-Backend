use std::collections::BTreeSet;

use crate::modules::chat::moderation::verdict::{Category, Verdict};

pub const REJECT_CONFIDENCE: f32 = 0.7;
pub const REMOVE_CONFIDENCE: f32 = 0.8;
pub const ILLEGAL_CONFIDENCE: f32 = 0.9;
pub const REPEAT_WINDOW_DAYS: i64 = 30;
pub const REPEAT_BAN_DAYS: i64 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ban {
    None,
    Days(i64),
    Permanent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Reject,
    Remove { seqs: Vec<i64>, ban: Ban },
    Fallback(&'static str),
    Later,
}

impl Decision {
    pub fn label(&self) -> &'static str {
        match self {
            Decision::Reject => "reject",
            Decision::Remove { .. } => "remove",
            Decision::Fallback(_) => "fallback",
            Decision::Later => "later",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Injection {
    pub outside_reporter: bool,
    pub outside_target: bool,
}

impl Injection {
    pub fn any(self) -> bool {
        self.outside_reporter || self.outside_target
    }
}

pub struct Facts {
    pub target_seqs: BTreeSet<i64>,
    pub injection: Injection,
    pub repeat: bool,
}

pub fn decide(verdict: Option<&Verdict>, facts: &Facts) -> Decision {
    let Some(verdict) = verdict else {
        return Decision::Fallback("no_verdict");
    };
    match judged(verdict, facts) {
        Decision::Reject if facts.injection.outside_reporter => Decision::Fallback("injection"),
        Decision::Remove { .. } if facts.injection.outside_target => {
            Decision::Fallback("injection")
        }
        decision => decision,
    }
}

fn judged(verdict: &Verdict, facts: &Facts) -> Decision {
    let quotes: BTreeSet<i64> = verdict.quote_seq.iter().copied().collect();
    if !quotes.is_subset(&facts.target_seqs) {
        return Decision::Fallback("quote_outside_report");
    }
    if !verdict.genuine {
        return if verdict.confidence >= REJECT_CONFIDENCE {
            Decision::Reject
        } else {
            Decision::Fallback("low_confidence")
        };
    }
    if quotes.is_empty() {
        return Decision::Fallback("no_quote");
    }
    let seqs: Vec<i64> = quotes.into_iter().collect();
    match verdict.category {
        Category::Illegal if verdict.confidence >= ILLEGAL_CONFIDENCE => Decision::Remove {
            seqs,
            ban: Ban::Permanent,
        },
        Category::Spam | Category::Harassment | Category::Doxxing
            if verdict.confidence >= REMOVE_CONFIDENCE =>
        {
            Decision::Remove {
                seqs,
                ban: if facts.repeat {
                    Ban::Days(REPEAT_BAN_DAYS)
                } else {
                    Ban::None
                },
            }
        }
        _ => Decision::Fallback("low_confidence"),
    }
}

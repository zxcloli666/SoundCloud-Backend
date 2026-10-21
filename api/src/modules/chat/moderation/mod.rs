pub mod actions;
pub mod client;
pub mod decide;
pub mod evidence;
pub mod history;
pub mod injection;
pub mod keys;
pub mod prompt;
pub mod rules;
pub mod task;
#[cfg(test)]
pub mod testing;
pub mod text;
pub mod verdict;

use std::collections::{BTreeSet, VecDeque};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::json;
use tokio::sync::{Notify, Semaphore};
use uuid::Uuid;

use crate::modules::chat::admin::audit;
use crate::modules::chat::moderation::client::{AiAnswer, AiClient, sha256_hex};
use crate::modules::chat::moderation::decide::Injection;
use crate::modules::chat::moderation::evidence::Line;
use crate::modules::chat::moderation::keys::ChatKeys;
use crate::modules::chat::moderation::prompt::Case;
use crate::modules::chat::moderation::verdict::Verdict;
use crate::modules::chat::reports::{self, ReportRow};
use crate::modules::chat::{ChatError, ChatResult, ChatState, requests};

pub const AUDIT_ADMIN: &str = "ai-moderator";
pub const CALLS_IN_FLIGHT: usize = 4;
pub const CALL_TIMEOUT_SECS: u64 = 30;
pub const BANS_PER_HOUR: usize = 10;
pub const BANS_PER_DAY: usize = 50;
const HOUR: Duration = Duration::from_secs(3_600);
const DAY: Duration = Duration::from_secs(86_400);

pub struct Moderator {
    client: AiClient,
    calls: Semaphore,
    wake: Notify,
    bans: Mutex<VecDeque<Instant>>,
}

pub struct Judgement {
    pub answer: AiAnswer,
    pub injection: Injection,
    pub target_seqs: BTreeSet<i64>,
}

impl Moderator {
    pub fn new(client: AiClient) -> Self {
        Self {
            client,
            calls: Semaphore::new(CALLS_IN_FLIGHT),
            wake: Notify::new(),
            bans: Mutex::new(VecDeque::with_capacity(BANS_PER_DAY)),
        }
    }

    pub fn allow_ban(&self) -> bool {
        let now = Instant::now();
        let mut bans = self.bans.lock().unwrap_or_else(PoisonError::into_inner);
        while bans
            .front()
            .is_some_and(|at| now.duration_since(*at) >= DAY)
        {
            bans.pop_front();
        }
        let last_hour = bans
            .iter()
            .filter(|at| now.duration_since(**at) < HOUR)
            .count();
        if bans.len() >= BANS_PER_DAY || last_hour >= BANS_PER_HOUR {
            return false;
        }
        bans.push_back(now);
        true
    }

    pub fn ai_enabled(&self) -> bool {
        self.client.enabled()
    }

    pub fn nudge(&self) {
        self.wake.notify_one();
    }

    pub async fn woken(&self) {
        self.wake.notified().await;
    }

    pub async fn judge(&self, chat: &ChatState, report: Uuid) -> ChatResult<Option<Verdict>> {
        let report = reports::load(chat, report)
            .await?
            .ok_or(ChatError::NotFound)?;
        Ok(self
            .examine(chat, &chat.keks, &report)
            .await?
            .answer
            .verdict)
    }

    pub async fn examine(
        &self,
        chat: &ChatState,
        keys: &dyn ChatKeys,
        report: &ReportRow,
    ) -> ChatResult<Judgement> {
        let lines = decrypt(chat, keys, report, AUDIT_ADMIN).await?;
        let target_seqs: BTreeSet<i64> = lines
            .iter()
            .filter(|line| line.reported && line.sender == report.target_user)
            .filter(|line| line.class != "system")
            .map(|line| line.seq)
            .collect();
        let injection = Injection {
            outside_reporter: injection::suspected_outside(&lines, &report.reporter),
            outside_target: injection::suspected_outside(&lines, &report.target_user)
                || injection::suspected(report.comment.as_deref()),
        };
        let seqs: Vec<i64> = target_seqs.iter().copied().collect();
        let case = Case {
            reason: &report.reason,
            comment: report.comment.as_deref(),
            reporter: &report.reporter,
            target: &report.target_user,
            lines: &lines,
            target_seqs: &seqs,
            reporter_history: history::reporter(chat, &report.reporter).await?,
            target_history: history::target(chat, &report.target_user).await?,
            contacts: requests::relation(chat, &report.reporter, &report.target_user)
                .await?
                .contact,
        };
        let prompt = prompt::build(&case, &prompt::fence());
        let prompt_sha256 = sha256_hex(format!("{}\n{}", prompt.system, prompt.user).as_bytes());
        audit::record(
            chat,
            AUDIT_ADMIN,
            "prompt",
            &audit::report(report.id),
            json!({
                "prompt_sha256": prompt_sha256,
                "model": self.client.model(),
                "lines": lines.len(),
            }),
        )
        .await?;
        let answer = {
            let _permit = self
                .calls
                .acquire()
                .await
                .map_err(|_| ChatError::Internal)?;
            self.client.verdict(&prompt).await
        };
        drop(prompt);
        drop(lines);
        metrics::counter!("chat_moderation_calls_total", "outcome" => answer.outcome.as_str())
            .increment(1);
        audit::record(
            chat,
            AUDIT_ADMIN,
            "verdict",
            &audit::report(report.id),
            json!({
                "outcome": answer.outcome.as_str(),
                "attempts": answer.attempts,
                "prompt_sha256": prompt_sha256,
                "answer_sha256": answer.answer_sha256,
                "verdict": answer.verdict,
                "injection": injection.any(),
                "injection_outside_reporter": injection.outside_reporter,
                "injection_outside_target": injection.outside_target,
            }),
        )
        .await?;
        Ok(Judgement {
            answer,
            injection,
            target_seqs,
        })
    }
}

pub async fn decrypt(
    chat: &ChatState,
    keys: &dyn ChatKeys,
    report: &ReportRow,
    by: &str,
) -> ChatResult<Vec<Line>> {
    let (Some(sealed), Some(version)) = (&report.evidence, report.evidence_kek_version) else {
        return Err(ChatError::NotFound);
    };
    let version = u16::try_from(version).map_err(|_| ChatError::Internal)?;
    audit::record(
        chat,
        by,
        "decrypt",
        &audit::report(report.id),
        json!({
            "conversation": report.conversation_id,
            "seq_from": report.seq_from,
            "seq_to": report.seq_to,
        }),
    )
    .await?;
    let bundle = evidence::open(keys, version, sealed)?;
    Ok(evidence::lines(&bundle))
}

#[cfg(test)]
#[path = "../tests/moderation_unit_tests.rs"]
mod unit_tests;

#[cfg(test)]
#[path = "../tests/moderation_tests.rs"]
mod tests;

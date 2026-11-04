use std::time::Duration;

use backend_contracts::remote::LinkProvider;
use uuid::Uuid;

use super::confirm::{LinkConfirm, MAX_MISSES, Status};
use super::hub::Asked;
use super::links::{FoundCode, LinksService};
use super::secrets::{random_hint, review_code_matches};
use crate::config::VoiceCfg;
use crate::error::AppResult;
use crate::state::AppState;

pub const REVIEW_USER: &str = "voice-review";

pub enum Found {
    NoCode,
    Capped,
    InUse,
    Ready(Pairing),
}

pub struct Pairing {
    pub code_id: Uuid,
    pub user: String,
    pub provider: LinkProvider,
    pub hint: String,
}

pub fn is_review_code(voice: &VoiceCfg, raw: &str) -> bool {
    review_code_matches(raw, voice.review_code.expose())
}

pub fn is_review_user(user: &str) -> bool {
    user == REVIEW_USER
}

pub async fn find_code_with_retry(
    links: &LinksService,
    provider: LinkProvider,
    code: &str,
    retry: Duration,
) -> AppResult<Option<FoundCode>> {
    if let Some(found) = links.find_code(provider, code).await? {
        return Ok(Some(found));
    }
    tokio::time::sleep(retry).await;
    links.find_code(provider, code).await
}

pub async fn find(st: &AppState, provider: LinkProvider, code: &str) -> AppResult<Found> {
    let retry = st.config.voice.code_retry;
    let Some(found) = find_code_with_retry(&st.links, provider, code, retry).await? else {
        return Ok(Found::NoCode);
    };
    if !st.confirm.claim(found.id).await? {
        return Ok(Found::InUse);
    }
    let asked = match st.confirm.asks_so_far(&found.user).await {
        Ok(count) => count,
        Err(error) => {
            st.confirm.release(found.id).await;
            return Err(error);
        }
    };
    if asked >= st.config.voice.confirm_per_user_day as i64 {
        st.confirm.release(found.id).await;
        return Ok(Found::Capped);
    }
    Ok(Found::Ready(Pairing {
        code_id: found.id,
        user: found.user,
        provider,
        hint: random_hint(),
    }))
}

pub async fn ask_desktop(st: &AppState, pairing: &Pairing) -> Status {
    let status = confirm_on_desktop(st, pairing).await;
    st.confirm.release(pairing.code_id).await;
    status
}

async fn confirm_on_desktop(st: &AppState, pairing: &Pairing) -> Status {
    if st.remote.active_device(&pairing.user).is_none() {
        return Status::Offline;
    }
    if let Err(error) = st.confirm.asked_today(&pairing.user).await {
        tracing::warn!(%error, "link confirmations were not counted");
        return Status::Failed;
    }
    let asked = st
        .remote
        .confirm_link(&pairing.user, pairing.provider, &pairing.hint)
        .await;
    match asked {
        Asked::Reply(reply) if reply.ok => match st.links.consume_code(pairing.code_id).await {
            Ok(true) => Status::Approved,
            _ => Status::Failed,
        },
        Asked::Reply(_) => {
            let _ = st.links.consume_code(pairing.code_id).await;
            Status::Denied
        }
        Asked::Offline => Status::Offline,
        Asked::Silent => {
            note_miss(&st.links, &st.confirm, pairing.code_id).await;
            Status::Silent
        }
    }
}

pub async fn note_miss(links: &LinksService, confirm: &LinkConfirm, code_id: Uuid) -> i64 {
    match confirm.missed(code_id).await {
        Ok(misses) => {
            if misses >= MAX_MISSES {
                let _ = links.consume_code(code_id).await;
            }
            misses
        }
        Err(error) => {
            tracing::warn!(%error, "link code misses were not counted");
            0
        }
    }
}

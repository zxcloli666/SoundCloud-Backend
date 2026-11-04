use std::time::Duration;

use backend_contracts::remote::LinkProvider;
use uuid::Uuid;

use super::confirm::{Confirming, MAX_CODE_ASKS, Status};
use super::hub::now_ms;
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
    match st.confirm.code_asks(found.id).await {
        Ok(asks) if asks >= MAX_CODE_ASKS => {
            let _ = st.links.consume_code(found.id).await;
            st.confirm.release(found.id).await;
            return Ok(Found::NoCode);
        }
        Ok(_) => {}
        Err(error) => {
            st.confirm.release(found.id).await;
            return Err(error);
        }
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

pub async fn ask(st: &AppState, pairing: &Pairing) -> Result<Confirming, Status> {
    let Some(command) = st
        .remote
        .link_confirm(&pairing.user, pairing.provider, &pairing.hint)
        .await
    else {
        st.confirm.release(pairing.code_id).await;
        return Err(Status::Offline);
    };
    if let Err(error) = st.confirm.asked_today(&pairing.user).await {
        tracing::warn!(%error, "link confirmations were not counted");
        st.confirm.release(pairing.code_id).await;
        return Err(Status::Failed);
    }
    if let Err(error) = st.confirm.note_code_ask(pairing.code_id).await {
        tracing::warn!(%error, "link code asks were not counted");
        st.confirm.release(pairing.code_id).await;
        return Err(Status::Failed);
    }
    if !st.remote.post(&command).await {
        st.confirm.release(pairing.code_id).await;
        return Err(Status::Failed);
    }
    Ok(Confirming {
        code_id: pairing.code_id,
        user: pairing.user.clone(),
        provider: pairing.provider,
        command: command.id,
        deadline_ms: command.expires_at,
    })
}

pub async fn outcome(st: &AppState, confirming: &Confirming) -> Option<Status> {
    if let Some(reply) = st.remote.answer(&confirming.user, confirming.command).await {
        let status = if reply.ok {
            match st.links.consume_code(confirming.code_id).await {
                Ok(true) => Status::Approved,
                _ => Status::Failed,
            }
        } else {
            let _ = st.links.consume_code(confirming.code_id).await;
            Status::Denied
        };
        st.confirm.release(confirming.code_id).await;
        return Some(status);
    }
    if now_ms() <= confirming.deadline_ms {
        return None;
    }
    st.confirm.release(confirming.code_id).await;
    Some(Status::Silent)
}

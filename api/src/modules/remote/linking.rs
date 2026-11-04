use backend_contracts::remote::LinkProvider;
use uuid::Uuid;

use super::confirm::{LinkConfirm, MAX_MISSES, Status};
use super::links::LinksService;
use super::hub::Asked;
use crate::error::AppResult;
use crate::state::AppState;

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
}

pub async fn find(st: &AppState, provider: LinkProvider, code: &str) -> AppResult<Found> {
    let Some(found) = st.links.find_code(provider, code).await? else {
        return Ok(Found::NoCode);
    };
    if !st.confirm.claim(found.id).await? {
        return Ok(Found::InUse);
    }
    let asked = st.confirm.asked_today(&found.user).await;
    let capped = match asked {
        Ok(count) => count > st.config.voice.confirm_per_user_day as i64,
        Err(error) => {
            st.confirm.release(found.id).await;
            return Err(error);
        }
    };
    if capped {
        st.confirm.release(found.id).await;
        return Ok(Found::Capped);
    }
    Ok(Found::Ready(Pairing {
        code_id: found.id,
        user: found.user,
        provider,
    }))
}

pub async fn ask_desktop(st: &AppState, pairing: &Pairing) -> Status {
    let asked = st.remote.confirm_link(&pairing.user, pairing.provider).await;
    let status = match asked {
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
    };
    st.confirm.release(pairing.code_id).await;
    status
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

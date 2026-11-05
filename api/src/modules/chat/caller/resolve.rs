use std::time::Duration;

use sc_chat_wire::proto::Inner;
use sc_chat_wire::sign::verify_request;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::modules::auth::service::replica::NOT_REPLICATED;
use crate::modules::chat::caller::{
    COLD_WAIT, CallerBind, CallerDevice, ChatCaller, Cold, REFRESH_WAIT, SessionEntry,
};
use crate::modules::chat::held::Held;
use crate::modules::chat::model::Tier;
use crate::modules::chat::{ChatError, ChatResult, ChatState, devices, synthetic};

pub const ANSWER_BYTES: u32 = 256 * 1024;
pub const PRO_ANSWER_BYTES: u32 = 24 * 1024;

pub struct Resolved {
    pub caller: ChatCaller,
    pub bind: Option<CallerBind>,
}

pub async fn resolve(chat: &ChatState, inner: &Inner, enc: &[u8; 32]) -> ChatResult<Resolved> {
    let session_id = parse(inner.session_id.as_deref()).ok_or(ChatError::ReloginRequired)?;
    let device_id = parse(inner.device_id.as_deref()).ok_or(ChatError::Invalid)?;
    let (session, bind) = signed_session(chat, session_id, device_id, inner, enc).await?;
    let device = match &bind {
        Some(bind) => device(chat, device_id, &session, bind).await?,
        None => None,
    };
    let tier = match inner.tier() {
        Tier::Unspecified => Tier::Direct,
        tier => tier,
    };
    Ok(Resolved {
        caller: ChatCaller {
            user_id: session.user_id,
            session_id,
            device,
            tier,
            max_bytes: max_bytes(tier, inner.max_bytes),
        },
        bind,
    })
}

async fn session_entry(
    chat: &ChatState,
    id: Uuid,
    wait: Duration,
) -> ChatResult<Option<SessionEntry>> {
    if chat.callers.is_unknown(id) {
        return Ok(None);
    }
    let found = chat
        .callers
        .cold(Cold::Session, wait, async {
            if let Some(synthetic) = synthetic::session(chat, id).await? {
                return Ok(Some(synthetic));
            }
            chat.auth
                .replicated_now(id, || stored_session(chat, id))
                .await
                .map_err(not_replicated)
        })
        .await?;
    if found.is_none() {
        chat.callers.remember_unknown(id);
    }
    Ok(found)
}

async fn stored_session(chat: &ChatState, id: Uuid) -> AppResult<Option<SessionEntry>> {
    let row = sqlx::query_file!("queries/auth/service/get_session_bind.sql", id)
        .fetch_optional(&chat.pg)
        .await?;
    Ok(row.map(|row| SessionEntry {
        user_id: row.soundcloud_user_id,
        bind: row.chat_bind.as_deref().and_then(CallerBind::decode),
    }))
}

fn not_replicated(error: AppError) -> ChatError {
    match error {
        AppError::Coded {
            code: NOT_REPLICATED,
            ..
        } => ChatError::SessionNotReplicated,
        AppError::Db(error) => ChatError::from(error),
        other => {
            tracing::error!(code = other.public_code(), "chat session lookup failed");
            ChatError::Internal
        }
    }
}

async fn signed_session(
    chat: &ChatState,
    session_id: Uuid,
    device_id: Uuid,
    inner: &Inner,
    enc: &[u8; 32],
) -> ChatResult<(SessionEntry, Option<CallerBind>)> {
    let looked = match held_signed(chat, session_id, device_id, inner, enc) {
        Some((held, bind)) if held.fresh() => return Ok((held.value, Some(bind))),
        Some((held, bind)) => match session_entry(chat, session_id, REFRESH_WAIT).await {
            Err(error) if error.is_busy() => {
                Cold::Session.count_stale();
                return Ok((held.value, Some(bind)));
            }
            looked => looked?,
        },
        None => session_entry(chat, session_id, COLD_WAIT).await?,
    };
    let Some(fresh) = looked else {
        chat.callers.forget_session(session_id);
        return Err(ChatError::ReloginRequired);
    };
    let bind = signed_by(&fresh, device_id, inner, enc)?;
    match bind {
        Some(_) => chat.callers.remember_session(session_id, fresh.clone()),
        None => chat.callers.forget_session(session_id),
    }
    Ok((fresh, bind))
}

fn held_signed(
    chat: &ChatState,
    session_id: Uuid,
    device_id: Uuid,
    inner: &Inner,
    enc: &[u8; 32],
) -> Option<(Held<SessionEntry>, CallerBind)> {
    let held = chat.callers.session(session_id)?;
    if synthetic::is_synthetic(&held.value.user_id) && !chat.config.loadtest_open() {
        return None;
    }
    let bind = signed_by(&held.value, device_id, inner, enc).ok()??;
    Some((held, bind))
}

fn signed_by(
    session: &SessionEntry,
    device_id: Uuid,
    inner: &Inner,
    enc: &[u8; 32],
) -> ChatResult<Option<CallerBind>> {
    let Some(bind) = session.bind else {
        return Ok(None);
    };
    if bind.device_id != device_id {
        return Err(ChatError::SessionBoundOther);
    }
    verify_request(&bind.auth_pub, inner, enc).map_err(|_| ChatError::Invalid)?;
    Ok(Some(bind))
}

async fn device(
    chat: &ChatState,
    device_id: Uuid,
    session: &SessionEntry,
    bind: &CallerBind,
) -> ChatResult<Option<CallerDevice>> {
    let Some(row) = devices::gated_row(chat, device_id).await? else {
        return Ok(None);
    };
    if row.user_id != session.user_id || row.auth_pub != bind.auth_pub {
        return Err(ChatError::SessionBoundOther);
    }
    if row.status != "active" {
        return Err(ChatError::DeviceRevoked);
    }
    if let Err(error) = devices::touch(chat, &row).await {
        tracing::debug!(
            code = error.code(),
            "a device last seen time was not written"
        );
    }
    Ok(Some(CallerDevice::from(&row)))
}

fn max_bytes(tier: Tier, asked: u32) -> u32 {
    let cap = match tier {
        Tier::Pro => PRO_ANSWER_BYTES,
        _ => ANSWER_BYTES,
    };
    match asked {
        0 => cap,
        asked => asked.min(cap),
    }
}

fn parse(raw: Option<&str>) -> Option<Uuid> {
    Uuid::parse_str(raw?).ok()
}

#[cfg(test)]
#[path = "../tests/resolve_tests.rs"]
mod tests;

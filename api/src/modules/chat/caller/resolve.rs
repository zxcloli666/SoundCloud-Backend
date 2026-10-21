use chat_wire::proto::Inner;
use chat_wire::sign::verify_request;
use uuid::Uuid;

use crate::modules::chat::caller::{CallerBind, CallerDevice, ChatCaller, SessionEntry};
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

async fn session_entry(chat: &ChatState, id: Uuid) -> ChatResult<Option<SessionEntry>> {
    chat.callers
        .cold(async {
            if let Some(synthetic) = synthetic::session(chat, id).await? {
                return Ok(Some(synthetic));
            }
            let row = sqlx::query_file!("queries/auth/service/get_session_bind.sql", id)
                .fetch_optional(&chat.pg)
                .await?;
            Ok(row.map(|row| SessionEntry {
                user_id: row.soundcloud_user_id,
                bind: row.chat_bind.as_deref().and_then(CallerBind::decode),
            }))
        })
        .await
}

async fn signed_session(
    chat: &ChatState,
    session_id: Uuid,
    device_id: Uuid,
    inner: &Inner,
    enc: &[u8; 32],
) -> ChatResult<(SessionEntry, Option<CallerBind>)> {
    if let Some(cached) = chat.callers.session(session_id)
        && let Ok(Some(bind)) = signed_by(&cached, device_id, inner, enc)
    {
        return Ok((cached, Some(bind)));
    }
    let Some(fresh) = session_entry(chat, session_id).await? else {
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
    let row = match chat.devices.row(device_id) {
        Some(row) => row,
        None => match chat.callers.cold(devices::lookup(chat, device_id)).await? {
            Some(row) => row,
            None => return Ok(None),
        },
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

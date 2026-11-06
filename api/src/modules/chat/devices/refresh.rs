use crate::modules::chat::devices::register::{APP_VERSION_MAX, CAP_MAX, CAPS_MAX, token};
use crate::modules::chat::devices::{RefreshDevice, changed, require, store};
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::model::DeviceRow;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub async fn refresh(
    chat: &ChatState,
    caller: &ChatCaller,
    request: RefreshDevice,
) -> ChatResult<DeviceRow> {
    validate(&request)?;
    let me = require(chat, caller).await?;
    limits::admit(chat, Bucket::DeviceRefresh, &caller.user_id).await?;
    if me.app_version == request.app_version && me.caps == request.caps {
        return Ok(me);
    }
    let Some(row) = store::refresh(&chat.pg, me.id, &request.app_version, &request.caps).await?
    else {
        return store::by_id(&chat.pg, me.id)
            .await?
            .ok_or(ChatError::DeviceRevoked);
    };
    changed(chat, &row.user_id, &[row.id]).await?;
    Ok(row)
}

fn validate(request: &RefreshDevice) -> ChatResult<()> {
    let fits = token(&request.app_version, APP_VERSION_MAX)
        && request.caps.len() <= CAPS_MAX
        && request.caps.iter().all(|cap| token(cap, CAP_MAX));
    if fits {
        Ok(())
    } else {
        Err(ChatError::Invalid)
    }
}

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::modules::chat::devices::bind::{BoundDevice, bound_device};
use crate::modules::chat::devices::store::{self, NewDevice, SessionMove};
use crate::modules::chat::devices::{RegisterDevice, changed, lookup};
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::model::{DEVICES_MAX, DeviceRow, Tier};
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState, users};

const PLATFORM_MAX: usize = 32;
const NAME_MAX: usize = 64;
const APP_VERSION_MAX: usize = 32;
const CAPS_MAX: usize = 16;
const CAP_MAX: usize = 32;

struct Enrolled {
    row: DeviceRow,
    evicted: Vec<Uuid>,
}

pub async fn register(
    chat: &ChatState,
    caller: &ChatCaller,
    request: RegisterDevice,
) -> ChatResult<DeviceRow> {
    let request = validated(request)?;
    let bound = session_device(chat, caller).await?;
    if lookup(chat, bound.id).await?.is_none() {
        limits::admit(chat, Bucket::Device, &caller.user_id).await?;
    }
    users::touch(chat, &caller.user_id).await?;
    let enrolled = enroll(chat, caller, &bound, &request).await?;
    let mut touched = enrolled.evicted;
    touched.push(enrolled.row.id);
    changed(chat, &caller.user_id, &touched).await?;
    Ok(enrolled.row)
}

async fn session_device(chat: &ChatState, caller: &ChatCaller) -> ChatResult<BoundDevice> {
    let session = store::session_bind(&chat.pg, caller.session_id)
        .await?
        .ok_or(ChatError::ReloginRequired)?;
    if session.soundcloud_user_id.as_deref() != Some(caller.user_id.as_str()) {
        return Err(ChatError::SessionBoundOther);
    }
    let bound = bound_device(session.chat_bind.as_deref())?;
    match &caller.device {
        Some(device) if device.id != bound.id => Err(ChatError::SessionBoundOther),
        _ => Ok(bound),
    }
}

async fn enroll(
    chat: &ChatState,
    caller: &ChatCaller,
    bound: &BoundDevice,
    request: &RegisterDevice,
) -> ChatResult<Enrolled> {
    let mut tx = chat.pg.begin().await?;
    store::lock_user(&mut *tx, &caller.user_id).await?;
    let known = store::lock_by_id(&mut *tx, bound.id).await?;
    if let Some(row) = &known {
        same_device(row, &caller.user_id, bound)?;
    }
    let fresh = known.is_none();
    let evicted = store::revoke_session_holders(&mut *tx, caller.session_id, bound.id).await?;
    let row = if fresh {
        first_login(&mut tx, caller, bound, request).await?
    } else {
        relogin(&mut tx, caller, bound, request).await?
    };
    if fresh || !evicted.is_empty() {
        store::rekey_user(&mut *tx, &caller.user_id).await?;
    }
    tx.commit().await?;
    Ok(Enrolled { row, evicted })
}

fn same_device(row: &DeviceRow, user_id: &str, bound: &BoundDevice) -> ChatResult<()> {
    if row.user_id != user_id || !bound.holds(&row.auth_pub, &row.box_pub) {
        return Err(ChatError::SessionBoundOther);
    }
    if row.status != "active" {
        return Err(ChatError::DeviceRevoked);
    }
    Ok(())
}

async fn relogin(
    tx: &mut Transaction<'_, Postgres>,
    caller: &ChatCaller,
    bound: &BoundDevice,
    request: &RegisterDevice,
) -> ChatResult<DeviceRow> {
    let change = SessionMove {
        id: bound.id,
        session_id: caller.session_id,
        platform: &request.platform,
        name: &request.name,
        app_version: &request.app_version,
        caps: &request.caps,
    };
    store::move_to_session(&mut **tx, &change)
        .await?
        .ok_or(ChatError::DeviceRevoked)
}

async fn first_login(
    tx: &mut Transaction<'_, Postgres>,
    caller: &ChatCaller,
    bound: &BoundDevice,
    request: &RegisterDevice,
) -> ChatResult<DeviceRow> {
    let live = store::count_live(&mut **tx, &caller.user_id).await?;
    if usize::try_from(live).unwrap_or(usize::MAX) >= DEVICES_MAX {
        return Err(ChatError::RequestLimit);
    }
    let device = NewDevice {
        id: bound.id,
        user_id: &caller.user_id,
        session_id: caller.session_id,
        auth_pub: &bound.auth_pub,
        box_pub: &bound.box_pub,
        platform: &request.platform,
        name: &request.name,
        app_version: &request.app_version,
        caps: &request.caps,
        tier: tier_name(caller.tier),
    };
    store::insert(&mut **tx, &device)
        .await?
        .ok_or(ChatError::SessionBoundOther)
}

fn validated(request: RegisterDevice) -> ChatResult<RegisterDevice> {
    let name = request.name.trim().to_owned();
    let fits = token(&request.platform, PLATFORM_MAX)
        && token(&request.app_version, APP_VERSION_MAX)
        && !name.is_empty()
        && name.chars().count() <= NAME_MAX
        && !name.chars().any(char::is_control)
        && request.caps.len() <= CAPS_MAX
        && request.caps.iter().all(|cap| token(cap, CAP_MAX));
    if !fits {
        return Err(ChatError::Invalid);
    }
    Ok(RegisterDevice { name, ..request })
}

fn token(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && value.bytes().all(|byte| byte.is_ascii_graphic())
}

fn tier_name(tier: Tier) -> &'static str {
    match tier {
        Tier::Relay => "relay",
        Tier::Pro => "pro",
        Tier::Direct | Tier::Unspecified => "direct",
    }
}

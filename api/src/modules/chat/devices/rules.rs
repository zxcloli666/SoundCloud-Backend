use chrono::{DateTime, Utc};

use crate::modules::chat::history_gate::seasoned;
use crate::modules::chat::model::DeviceRow;

pub fn may_remove(actor: &DeviceRow, target: &DeviceRow, now: DateTime<Utc>) -> bool {
    younger(target, actor) || seasoned(actor.created_at, now)
}

pub fn may_disown(actor: &DeviceRow, target: &DeviceRow) -> bool {
    younger(target, actor)
}

pub fn may_confirm(actor: &DeviceRow, target: &DeviceRow, now: DateTime<Utc>) -> bool {
    younger(target, actor) && seasoned(actor.created_at, now)
}

fn younger(device: &DeviceRow, than: &DeviceRow) -> bool {
    device.created_at > than.created_at
}

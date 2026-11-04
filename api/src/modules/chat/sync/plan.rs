use std::time::Duration;

use crate::modules::chat::model::Tier;
use crate::modules::chat::sync::wire::{
    ANSWER_BYTES_MAX, ANSWER_BYTES_MAX_PRO, HOLD_MAX_MS, HOLD_MAX_PRO_MS, SyncRequest,
};
use crate::modules::chat::{ChatCaller, ChatConfig};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub hold: Duration,
    pub budget: usize,
}

impl Plan {
    pub fn new(config: &ChatConfig, caller: &ChatCaller, request: &SyncRequest) -> Self {
        let pro = caller.tier == Tier::Pro;
        let hold_cap = if pro { HOLD_MAX_PRO_MS } else { HOLD_MAX_MS };
        let bytes_cap = if pro {
            ANSWER_BYTES_MAX_PRO
        } else {
            ANSWER_BYTES_MAX
        };
        let mut budget = request.max_bytes.min(bytes_cap);
        if caller.max_bytes > 0 {
            budget = budget.min(caller.max_bytes);
        }
        Self {
            hold: Duration::from_millis(request.hold_ms.min(config.hold_max_ms).min(hold_cap)),
            budget: budget as usize,
        }
    }
}

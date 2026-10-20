pub mod devices;
pub mod expired;
pub mod leader;
pub mod marks;
pub mod purge;
pub mod reports;
pub mod rooms;
pub mod startup;

use std::sync::Arc;

use crate::modules::chat::ChatState;

pub fn spawn(_chat: Arc<ChatState>) {}

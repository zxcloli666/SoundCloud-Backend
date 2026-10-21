pub mod budget;
pub mod devices;
pub mod expired;
pub mod gauges;
pub mod hooks;
pub mod leader;
pub mod marks;
pub mod purge;
pub mod reports;
pub mod rooms;
pub mod schedule;
pub mod startup;

use std::sync::Arc;

use crate::modules::chat::ChatState;

pub fn spawn(chat: Arc<ChatState>) {
    tokio::spawn(leader::keep(chat.clone()));
    tokio::spawn(schedule::run(chat));
}

#[cfg(test)]
#[path = "../tests/sweeps_tests/mod.rs"]
mod tests;

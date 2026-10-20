#![allow(dead_code)]

pub mod admin;
pub mod auth_bind;
pub mod b64;
pub mod blocks;
pub mod body;
pub mod caller;
pub mod channel;
pub mod config;
pub mod conversations;
pub mod devices;
pub mod epochs;
pub mod errors;
pub mod history_gate;
pub mod hot_cache;
pub mod hub;
pub mod invites;
pub mod kek;
pub mod keys;
pub mod limits;
pub mod members;
pub mod messages;
pub mod model;
pub mod moderation;
pub mod presence;
pub mod reports;
pub mod requests;
pub mod rooms;
pub mod routes;
pub mod rt_keys;
pub mod settings;
pub mod state;
pub mod sweeps;
pub mod sync;
pub mod synthetic;
#[cfg(test)]
pub mod test_support;
pub mod typing;
pub mod users;

pub use caller::ChatCaller;
pub use config::ChatConfig;
pub use errors::{ChatError, ChatResult};
pub use state::ChatState;

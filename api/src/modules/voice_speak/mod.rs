mod handlers;
mod phrases;
mod reply;
mod request;
mod service;
mod stores;
mod streams;
#[cfg(test)]
pub(crate) mod testing;

pub use handlers::router;
pub use service::VoiceSpeak;

mod handlers;
mod objects;
mod phrases;
mod request;
mod service;
#[cfg(test)]
pub(crate) mod testing;
mod voicing;

pub use handlers::router;
pub use service::VoiceSpeak;

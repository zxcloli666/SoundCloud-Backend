pub mod handlers;
pub mod lyrics;
pub mod query;
pub mod repository;
pub mod semantic;
pub mod service;
pub mod vibe;

#[cfg(test)]
mod user_tests;

#[cfg(test)]
mod catalog_tests;

#[cfg(test)]
mod stampede_tests;

#[cfg(test)]
mod vibe_live_tests;

#[cfg(test)]
mod vibe_lyrics_live_tests;

pub use handlers::router;
pub use semantic::VibeSearchService;
pub use service::SearchService;

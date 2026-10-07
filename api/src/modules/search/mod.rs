pub mod catalog;
pub mod failure;
pub mod handlers;
pub mod lyrics;
pub mod semantic;
pub mod terms;
pub mod vibe;

#[cfg(test)]
mod catalog_tests;

#[cfg(test)]
mod failure_tests;

#[cfg(test)]
pub(crate) mod lexicon_refresh;

#[cfg(test)]
mod lyrics_tests;

#[cfg(test)]
mod plan_tests;

#[cfg(test)]
mod stampede_tests;

#[cfg(test)]
mod terms_tests;

#[cfg(test)]
mod vibe_live_tests;

pub use catalog::SearchService;
pub use handlers::router;
pub use semantic::VibeSearchService;

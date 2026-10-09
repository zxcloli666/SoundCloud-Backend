pub mod handlers;
pub mod service;

pub use handlers::router;
pub use service::IndexingService;

#[cfg(test)]
mod held_tests;

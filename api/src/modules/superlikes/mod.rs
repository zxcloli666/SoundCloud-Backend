pub mod handlers;
pub mod order;
pub mod page;
pub mod service;

pub use handlers::router;

pub const SUPERLIKE_WEIGHT: f32 = backend_contracts::SUPERLIKE_WEIGHT as f32;

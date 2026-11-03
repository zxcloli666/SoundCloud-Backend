pub mod handlers;
pub mod service;

pub use handlers::router;
pub use service::SubscriptionsService;

#[cfg(test)]
mod service_tests;

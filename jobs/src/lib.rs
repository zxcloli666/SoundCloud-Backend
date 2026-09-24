pub mod queue;

pub type ClientBuildError = anyhow::Error;

mod app;
mod bus;
mod config;
mod db;
#[cfg(test)]
#[path = "env_surface_tests.rs"]
mod env_surface_tests;
mod handlers;
mod health;
mod metrics;
mod migration_app;
mod qdrant;
#[cfg(test)]
#[path = "query_surface_tests.rs"]
mod query_surface_tests;
mod scheduler;
#[cfg(test)]
#[path = "secret_surface_tests.rs"]
mod secret_surface_tests;
mod supervisor;
mod telemetry;

pub use app::run;
pub use migration_app::{MigrationScope, run as run_migrations};
pub use telemetry::init as init_telemetry;

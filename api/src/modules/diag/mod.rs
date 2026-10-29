mod code;
mod handlers;
mod jaeger;
mod limits;
mod service;

pub use handlers::router;
pub use service::DiagService;

#[cfg(test)]
mod tests;

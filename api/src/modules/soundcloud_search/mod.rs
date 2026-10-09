pub(crate) mod project;
pub(crate) mod service;

#[cfg(test)]
mod tests;

pub(crate) use project::project_page;
pub use service::SoundCloudSearch;

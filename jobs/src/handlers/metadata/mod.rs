mod normalizer;
mod phrases;
mod quote;
mod text;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

pub use normalizer::Normalizer;
pub use quote::{quoted, quoted_list};
pub use text::{casefold, find_name, is_reupload_channel, unique, words};

pub fn share(value: f64) -> Option<f64> {
    (value.is_finite() && (0.0..=1.0).contains(&value)).then_some(value)
}

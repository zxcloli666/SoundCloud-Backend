mod kinds;
#[cfg(feature = "sqlx")]
mod sqlx_pg;
#[cfg(feature = "tokio-postgres")]
pub mod tokio_pg;

pub use kinds::Kind;
#[cfg(feature = "sqlx")]
pub use sqlx_pg::call;

pub const RACE_HINT: &str = "scd_wt: retry the transaction, a concurrent apply inserted this key";

pub trait Raced {
    fn raced(&self) -> bool;
}

pub async fn retry<T, E: Raced, F: Future<Output = Result<T, E>>>(
    mut transaction: impl FnMut() -> F,
) -> Result<T, E> {
    match transaction().await {
        Err(error) if error.raced() => transaction().await,
        result => result,
    }
}

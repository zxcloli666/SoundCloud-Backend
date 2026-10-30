use std::future::Future;
use std::time::Duration;

use rand::Rng;

use crate::error::{AppError, AppResult};

const ATTEMPTS: u32 = 3;

pub async fn retry_conflicts<T, F, Fut>(mut transaction: F) -> AppResult<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = AppResult<T>>,
{
    let mut attempt = 1;
    loop {
        match transaction().await {
            Err(error) if attempt < ATTEMPTS && conflicted(&error) => {
                let jitter = rand::thread_rng().gen_range(0..20);
                tokio::time::sleep(Duration::from_millis(u64::from(attempt * 15) + jitter)).await;
                attempt += 1;
            }
            result => return result,
        }
    }
}

fn conflicted(error: &AppError) -> bool {
    let AppError::Db(error) = error else {
        return false;
    };
    error
        .as_database_error()
        .and_then(|error| error.code())
        .is_some_and(|code| code == "40P01" || code == "40001")
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;

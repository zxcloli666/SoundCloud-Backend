use std::future::Future;
use std::time::Duration;

const ATTEMPTS: u32 = 3;

pub async fn retry_conflicts<T, F, Fut>(mut transaction: F) -> Result<T, sqlx::Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, sqlx::Error>>,
{
    let mut attempt = 1;
    loop {
        match transaction().await {
            Err(error) if attempt < ATTEMPTS && conflicted(&error) => {
                tokio::time::sleep(Duration::from_millis(u64::from(attempt) * 20)).await;
                attempt += 1;
            }
            result => return result,
        }
    }
}

fn conflicted(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .is_some_and(|code| code == "40P01" || code == "40001")
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;

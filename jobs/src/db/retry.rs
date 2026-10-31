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
            Err(error) if attempt < ATTEMPTS && wt::conflicted(&error) => {
                tokio::time::sleep(Duration::from_millis(u64::from(attempt) * 20)).await;
                attempt += 1;
            }
            result => return result,
        }
    }
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;

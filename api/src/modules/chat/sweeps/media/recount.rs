use crate::modules::chat::{ChatResult, ChatState};

pub const PAGE: i64 = 1_000;

#[derive(Debug, Default)]
pub struct Page {
    pub users: u64,
    pub drift: i64,
    pub last: Option<String>,
    pub full: bool,
}

pub async fn page(chat: &ChatState, after: &str) -> ChatResult<Page> {
    let users =
        sqlx::query_file_scalar!("queries/chat/sweeps/media_recount_users.sql", after, PAGE)
            .fetch_all(&chat.pg)
            .await?;
    if users.is_empty() {
        return Ok(Page::default());
    }
    let mut tx = chat.pg.begin().await?;
    sqlx::query_file_scalar!("queries/chat/sweeps/media_recount_lock.sql", &users)
        .fetch_all(&mut *tx)
        .await?;
    let drift = sqlx::query_file_scalar!("queries/chat/sweeps/media_recount.sql", &users)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Page {
        users: users.len() as u64,
        drift,
        full: users.len() as i64 == PAGE,
        last: users.into_iter().last(),
    })
}

#[cfg(test)]
pub async fn all(chat: &ChatState) -> ChatResult<i64> {
    let mut after = String::new();
    let mut drift = 0;
    loop {
        let page = page(chat, &after).await?;
        drift += page.drift;
        match page.last {
            Some(last) if page.full => after = last,
            _ => return Ok(drift),
        }
    }
}

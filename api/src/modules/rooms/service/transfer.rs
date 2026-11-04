use super::RoomsService;
use crate::error::AppResult;
use crate::modules::rooms::store::{Moved, Page};

impl RoomsService {
    pub async fn peek_page(&self, cursor: &str, count: usize) -> AppResult<Page> {
        self.store.peek_page(cursor, count).await
    }

    pub async fn ack_page(&self, keys: &[String]) -> AppResult<usize> {
        self.store.ack_page(keys).await
    }

    pub async fn put_page(&self, moved: &[Moved]) -> AppResult<usize> {
        self.store.put_page(moved).await
    }
}

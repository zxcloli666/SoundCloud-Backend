use super::RoomsService;
use crate::error::AppResult;
use crate::modules::rooms::store::{Moved, Page};

impl RoomsService {
    pub async fn take_page(&self, cursor: &str, count: usize) -> AppResult<Page> {
        self.store.take_page(cursor, count).await
    }

    pub async fn put_page(&self, moved: &[Moved]) -> AppResult<usize> {
        self.store.put_page(moved).await
    }
}

use std::sync::Arc;

use mini_moka::sync::Cache;

use crate::modules::chat::media::limits::{STAR_NO_TTL, STAR_YES_TTL};
use crate::modules::chat::{ChatError, ChatResult};
use crate::modules::subscriptions::SubscriptionsService;

const CAPACITY: u64 = 100_000;

pub struct Stars {
    service: Arc<SubscriptionsService>,
    yes: Cache<String, ()>,
    no: Cache<String, ()>,
}

impl Stars {
    pub fn new(service: Arc<SubscriptionsService>) -> Self {
        Self {
            service,
            yes: Cache::builder()
                .max_capacity(CAPACITY)
                .time_to_live(STAR_YES_TTL)
                .build(),
            no: Cache::builder()
                .max_capacity(CAPACITY)
                .time_to_live(STAR_NO_TTL)
                .build(),
        }
    }

    pub async fn is_star(&self, user_id: &str) -> ChatResult<bool> {
        let key = user_id.to_owned();
        if self.yes.contains_key(&key) {
            return Ok(true);
        }
        if self.no.contains_key(&key) {
            return Ok(false);
        }
        let premium = self.service.is_premium(user_id).await.map_err(|error| {
            tracing::warn!(%error, "the STAR check failed");
            ChatError::busy()
        })?;
        if premium {
            self.yes.insert(key, ());
        } else {
            self.no.insert(key, ());
        }
        Ok(premium)
    }
}

#[cfg(test)]
#[path = "../tests/media_star_tests.rs"]
mod tests;

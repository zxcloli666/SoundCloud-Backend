use std::time::Duration;

use sc_transport::SearchType;
use serde_json::Value;

use super::ScReadService;
use crate::error::{AppError, AppResult};
use crate::sc::within_budget;

impl ScReadService {
    pub async fn relay_usable(&self) -> bool {
        self.sc.has_relay() && !self.lua_health.is_open().await
    }

    pub async fn search_relay(
        &self,
        ty: SearchType,
        q: &str,
        limit: i64,
        wait: Duration,
    ) -> Option<Vec<Value>> {
        if !self.relay_usable().await {
            return None;
        }
        let call = async {
            match tokio::time::timeout(wait, self.sc.search_via_relay(ty.as_str(), q, None, limit))
                .await
            {
                Ok(Some(page)) => Ok(page),
                Ok(None) => Err(AppError::ScUnreachable("relay: no search result".into())),
                Err(_) => Err(AppError::sc_deadline_exceeded()),
            }
        };
        let page = Self::timed("relay_lua", "search", call).await.ok()?;
        let items = page.get("collection").and_then(Value::as_array)?;
        Some(normalized(items.clone()))
    }

    pub async fn search_proxy(
        &self,
        ty: SearchType,
        q: &str,
        limit: i64,
        wait: Duration,
    ) -> AppResult<Vec<Value>> {
        let call = within_budget(wait, async {
            self.proxy
                .search_page(ty, q, None, limit)
                .await
                .map_err(AppError::from)
        });
        let page = Self::timed("backup", "search", call).await?;
        Ok(normalized(page.items))
    }
}

fn normalized(items: Vec<Value>) -> Vec<Value> {
    items
        .into_iter()
        .map(|mut item| {
            sc_transport::normalize_v2_to_v1(&mut item);
            item
        })
        .collect()
}

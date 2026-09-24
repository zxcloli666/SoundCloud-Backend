use serde_json::Value;
use tracing::{debug, warn};

use sc_transport::{
    Apiv2Proxy, EGRESS_RELAY_LUA, EGRESS_RELAY_RAW, EgressHealth, ScClient, ScResult, SearchType,
    normalize_v2_to_v1, parse_list_cursor,
};

use super::egress::{EGRESS_APP, PgEgressHealth};

const SC_API_V2: &str = "https://api-v2.soundcloud.com";

pub struct CollectionPage {
    pub items: Vec<Value>,
    pub next_href: Option<String>,
}

pub struct PublicCatalogReader {
    sc: ScClient,
    proxy: Apiv2Proxy,
    lua_health: EgressHealth,
    raw_health: EgressHealth,
}

impl PublicCatalogReader {
    pub fn new(sc: ScClient, pg: sqlx::PgPool) -> Self {
        let store = PgEgressHealth::new(pg);
        Self {
            proxy: Apiv2Proxy::new(sc.clone()),
            sc,
            lua_health: EgressHealth::new(EGRESS_RELAY_LUA, EGRESS_APP, Some(store.clone())),
            raw_health: EgressHealth::new(EGRESS_RELAY_RAW, EGRESS_APP, Some(store)),
        }
    }

    async fn lua_is_worth_trying(&self) -> bool {
        !self.lua_health.is_open().await
    }

    async fn raw_is_worth_trying(&self) -> bool {
        !self.raw_health.is_open().await
    }

    async fn observe_lua<T>(&self, read: &sc_transport::RelayRead<T>) {
        if self.lua_health.observe(read).await {
            debug!("relay lua breaker is open, background reads take the proxy");
        }
    }

    async fn observe_raw_answer(&self, answered: bool) {
        if self.raw_health.record_answer(answered).await {
            debug!("relay raw breaker is open, background reads take the proxy");
        }
    }

    async fn observe_lua_answer(&self, answered: bool) {
        if self.lua_health.record_answer(answered).await {
            debug!("relay lua breaker is open, background reads take the proxy");
        }
    }

    pub async fn user_by_id(&self, user_id: &str) -> ScResult<Value> {
        if self.lua_is_worth_trying().await {
            let read = self.sc.user_by_id_via_relay(user_id).await;
            self.observe_lua(&read).await;
            if let Some(mut user) = read.found() {
                normalize_v2_to_v1(&mut user);
                return Ok(user);
            }
        }
        self.proxy.user(user_id).await
    }

    pub async fn resolve_url(&self, url: &str) -> ScResult<Value> {
        let path = format!("/resolve?url={}", urlencoding::encode(url));
        self.get_json(&path).await
    }

    pub async fn get_json(&self, path: &str) -> ScResult<Value> {
        self.get_json_from_region(path, 0).await
    }

    pub async fn get_json_from_region(&self, path: &str, region_rotation: i32) -> ScResult<Value> {
        let url = format!("{SC_API_V2}{path}");
        if self.raw_is_worth_trying().await {
            let answer = self
                .sc
                .apiv2_get_via_relay_rotated(&url, region_rotation)
                .await;
            self.observe_raw_answer(answer.is_some()).await;
            if let Some(value) = answer {
                return Ok(value);
            }
            debug!(
                path,
                region_rotation, "relay read unavailable, falling back to the proxy"
            );
        }
        self.proxy.get_value(&url).await
    }

    pub async fn search_tracks(&self, query: &str, limit: i64) -> ScResult<Vec<Value>> {
        if self.lua_is_worth_trying().await {
            let answer = self
                .sc
                .search_via_relay(SearchType::Tracks.as_str(), query, None, limit)
                .await;
            self.observe_lua_answer(answer.is_some()).await;
            if let Some(page) = answer {
                return Ok(page_from_relay(&page).items);
            }
            debug!(query, "relay search unavailable, falling back to the proxy");
        }
        let page = self
            .proxy
            .search_page(SearchType::Tracks, query, None, limit)
            .await?;
        let mut items = page.items;
        for item in items.iter_mut() {
            normalize_v2_to_v1(item);
        }
        Ok(items)
    }

    pub async fn list_page(
        &self,
        path: &str,
        cursor: Option<&str>,
        limit: i64,
        region_rotation: i32,
    ) -> ScResult<CollectionPage> {
        let url = match cursor {
            Some(cursor) => {
                parse_list_cursor(cursor)?;
                cursor.to_owned()
            }
            None => {
                let separator = if path.contains('?') { '&' } else { '?' };
                format!("{SC_API_V2}{path}{separator}limit={limit}&linked_partitioning=true")
            }
        };
        if self.raw_is_worth_trying().await {
            let answer = self
                .sc
                .apiv2_get_via_relay_rotated(&url, region_rotation)
                .await;
            self.observe_raw_answer(answer.is_some()).await;
            if let Some(page) = answer {
                return Ok(page_from_relay(&page));
            }
            debug!(path, "relay listing unavailable, falling back to the proxy");
        }
        let page = self.proxy.get_list(&url).await?;
        let mut items = page.items;
        for item in items.iter_mut() {
            normalize_v2_to_v1(item);
        }
        Ok(CollectionPage {
            items,
            next_href: page.next_href,
        })
    }
}

fn page_from_relay(page: &Value) -> CollectionPage {
    let items = page
        .get("collection")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .cloned()
                .map(|mut item| {
                    normalize_v2_to_v1(&mut item);
                    item
                })
                .collect()
        })
        .unwrap_or_default();
    CollectionPage {
        items,
        next_href: accepted_cursor(page),
    }
}

fn accepted_cursor(page: &Value) -> Option<String> {
    let cursor = page
        .get("next_href")
        .and_then(Value::as_str)
        .filter(|cursor| !cursor.is_empty())?;
    match parse_list_cursor(cursor) {
        Ok(_) => Some(cursor.to_owned()),
        Err(_) => {
            warn!("relay listing offered a next page that is not SoundCloud's, stopping here");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct SilentRelay {
        calls: Arc<AtomicUsize>,
    }

    impl sc_transport::RelayTransport for SilentRelay {
        fn fetch<'a>(
            &'a self,
            _request: &'a call_relay::Request,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<call_relay::Response, call_relay::Error>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async { Err(call_relay::Error::Disabled) })
        }

        fn call_method_rotated<'a>(
            &'a self,
            _method_id: &'a str,
            _script: &'a str,
            _inputs: sc_transport::Bytes,
            _region_rotation: i32,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<sc_transport::Bytes, call_relay::Error>>
                    + Send
                    + 'a,
            >,
        > {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(call_relay::Error::Disabled) })
        }
    }

    fn unreachable_pool() -> sqlx::PgPool {
        sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(50))
            .connect_lazy("postgres://nobody:nobody@127.0.0.1:1/nothing")
            .expect("a lazy pool never dials")
    }

    #[tokio::test]
    async fn a_relay_that_keeps_silent_stops_receiving_background_reads_even_without_postgres() {
        let calls = Arc::new(AtomicUsize::new(0));
        let sc = ScClient::new(&sc_transport::ScConfig {
            proxy_url: String::new(),
            proxy_fallback: false,
            api_base: Some("http://127.0.0.1:1".to_owned()),
            home_base: Some("http://127.0.0.1:1".to_owned()),
        })
        .expect("builds")
        .with_relay(Arc::new(SilentRelay {
            calls: calls.clone(),
        }));
        let reader = PublicCatalogReader::new(sc, unreachable_pool());

        for _ in 0..12 {
            let _ = reader.user_by_id("17").await;
        }

        let reached = calls.load(Ordering::SeqCst);
        assert!(reached > 0, "the first reads must actually try the relay");
        assert!(
            reached < 12,
            "a relay that answers nobody must stop receiving background reads, saw {reached} of 12"
        );
    }

    #[test]
    fn a_next_page_that_points_away_from_soundcloud_is_not_followed() {
        for hostile in [
            "http://127.0.0.1:8080/collection",
            "http://169.254.169.254/latest/meta-data/",
            "https://api-v2.soundcloud.com@evil.example/users/17/tracks",
            "https://evil.example/users/17/tracks",
            "file:///etc/passwd",
        ] {
            let page = serde_json::json!({"collection": [], "next_href": hostile});
            assert_eq!(
                accepted_cursor(&page),
                None,
                "{hostile} came back inside an answer relayed from a node we do not own, and \
                 the very next listing would dial it from inside our network"
            );
        }
    }

    #[test]
    fn the_page_soundcloud_actually_offers_is_still_followed() {
        let page = serde_json::json!({
            "collection": [],
            "next_href": "https://api-v2.soundcloud.com/users/17/tracks?offset=40",
        });
        assert_eq!(
            accepted_cursor(&page).as_deref(),
            Some("https://api-v2.soundcloud.com/users/17/tracks?offset=40"),
            "refusing every cursor would silently truncate every walk to its first page"
        );
    }

    #[tokio::test]
    async fn a_cursor_handed_to_the_reader_is_checked_before_anything_dials_it() {
        let calls = Arc::new(AtomicUsize::new(0));
        let sc = ScClient::new(&sc_transport::ScConfig {
            proxy_url: String::new(),
            proxy_fallback: false,
            api_base: Some("http://127.0.0.1:1".to_owned()),
            home_base: Some("http://127.0.0.1:1".to_owned()),
        })
        .expect("builds")
        .with_relay(Arc::new(SilentRelay {
            calls: calls.clone(),
        }));
        let reader = PublicCatalogReader::new(sc, unreachable_pool());

        let refused = reader
            .list_page("/users/17/tracks", Some("http://127.0.0.1:8080/x"), 20, 0)
            .await;

        assert!(refused.is_err());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "the reader must refuse the address itself, not rely on whoever stored it; an \
             unreachable relay makes every listing fail anyway, so failing is not the proof"
        );

        let _ = reader
            .list_page(
                "/users/17/tracks",
                Some("https://api-v2.soundcloud.com/users/17/tracks?offset=40"),
                20,
                0,
            )
            .await;

        assert!(
            calls.load(Ordering::SeqCst) > 0,
            "a check that refuses SoundCloud's own next page would truncate every walk to one \
             page, and this guard would still pass"
        );
    }

    #[tokio::test]
    async fn the_two_relay_tiers_carry_their_own_breaker() {
        let sc = ScClient::new(&sc_transport::ScConfig {
            proxy_url: String::new(),
            proxy_fallback: false,
            api_base: Some("http://127.0.0.1:1".to_owned()),
            home_base: Some("http://127.0.0.1:1".to_owned()),
        })
        .expect("builds");
        let reader = PublicCatalogReader::new(sc, unreachable_pool());

        for _ in 0..8 {
            reader.observe_lua_answer(false).await;
        }

        assert!(
            !reader.lua_is_worth_trying().await,
            "a dead lua tier must stop being tried"
        );
        assert!(
            reader.raw_is_worth_trying().await,
            "a dead lua tier must not close the raw apiv2 tier, they are different egress points"
        );
    }
}

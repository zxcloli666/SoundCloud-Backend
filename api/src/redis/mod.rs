use std::time::Duration;

use deadpool_redis::{Config, Pool, PoolConfig, Runtime, Timeouts};

use crate::config::AppConfig;

const POOL_WAIT: Duration = Duration::from_millis(500);
const POOL_CREATE: Duration = Duration::from_secs(1);
const POOL_RECYCLE: Duration = Duration::from_secs(1);

fn bounded(pool: Option<PoolConfig>) -> PoolConfig {
    let mut pool = pool.unwrap_or_default();
    pool.timeouts = Timeouts {
        wait: Some(POOL_WAIT),
        create: Some(POOL_CREATE),
        recycle: Some(POOL_RECYCLE),
    };
    pool
}

pub fn connect(cfg: &AppConfig) -> Result<Pool, deadpool_redis::CreatePoolError> {
    let mut rcfg = Config::from_url(&cfg.redis.url);
    rcfg.pool = Some(bounded(rcfg.pool.take()));
    rcfg.create_pool(Some(Runtime::Tokio1))
}

pub fn connect_admission(cfg: &AppConfig) -> Result<Pool, deadpool_redis::CreatePoolError> {
    let mut rcfg = Config::from_url(&cfg.redis.url);
    rcfg.pool = Some(bounded(Some(PoolConfig::new(cfg.admission.max_in_flight))));
    rcfg.create_pool(Some(Runtime::Tokio1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pool_bounds_the_wait_for_a_connection() {
        let pool = bounded(None);
        assert_eq!(pool.timeouts.wait, Some(POOL_WAIT));
        assert_eq!(pool.timeouts.create, Some(POOL_CREATE));
        assert_eq!(pool.timeouts.recycle, Some(POOL_RECYCLE));
    }

    #[test]
    fn bounding_keeps_an_explicit_pool_size() {
        let pool = bounded(Some(PoolConfig::new(7)));
        assert_eq!(pool.max_size, 7);
        assert_eq!(pool.timeouts.wait, Some(POOL_WAIT));
    }

    async fn a_socket_that_never_answers() -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port is free");
        let address = listener.local_addr().expect("the listener has an address");
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });
        address
    }

    fn pool_at(address: std::net::SocketAddr) -> Pool {
        let mut rcfg = Config::from_url(format!("redis://{address}"));
        rcfg.pool = Some(bounded(Some(PoolConfig::new(2))));
        rcfg.create_pool(Some(Runtime::Tokio1))
            .expect("a lazy pool never dials")
    }

    fn pool_at_without_a_connect_budget(address: std::net::SocketAddr) -> Pool {
        let mut rcfg = Config::from_url(format!("redis://{address}"));
        let mut pool = PoolConfig::new(2);
        pool.timeouts = Timeouts {
            wait: Some(POOL_WAIT),
            create: None,
            recycle: Some(POOL_RECYCLE),
        };
        rcfg.pool = Some(pool);
        rcfg.create_pool(Some(Runtime::Tokio1))
            .expect("a lazy pool never dials")
    }

    const BUDGET: Duration = Duration::from_secs(4);

    #[tokio::test]
    async fn a_redis_that_accepts_and_goes_quiet_does_not_hold_the_request() {
        let cache = crate::cache::CacheService::new(pool_at(a_socket_that_never_answers().await));

        let read = tokio::time::timeout(BUDGET, cache.get_raw("anything")).await;

        assert!(
            read.is_ok(),
            "a Redis that opened the socket and then said nothing held the read for {BUDGET:?}; \
             every request on this path would wait with it"
        );
    }

    #[tokio::test]
    async fn a_write_to_a_silent_redis_gives_up_as_well() {
        let cache = crate::cache::CacheService::new(pool_at(a_socket_that_never_answers().await));

        let written = tokio::time::timeout(
            BUDGET,
            cache.set_raw(
                "anything",
                "value",
                60,
                None,
                crate::cache::cache_service::CacheScope::Shared,
                None,
            ),
        )
        .await;

        assert!(
            written.is_ok(),
            "a write to a silent Redis held the request for {BUDGET:?}; a cache write must \
             never be able to outlive the answer it was meant to speed up"
        );
    }

    #[tokio::test]
    async fn the_cache_bounds_itself_and_not_only_through_the_pool() {
        let cache = crate::cache::CacheService::new(pool_at_without_a_connect_budget(
            a_socket_that_never_answers().await,
        ));

        let read = tokio::time::timeout(BUDGET, cache.get_raw("anything")).await;
        let written = tokio::time::timeout(
            BUDGET,
            cache.set_raw(
                "anything",
                "value",
                60,
                None,
                crate::cache::cache_service::CacheScope::Shared,
                None,
            ),
        )
        .await;

        assert!(
            read.is_ok() && written.is_ok(),
            "with the pool's connect budget removed the cache still has to give up on its own; \
             otherwise the whole fail-open rests on a setting in another module"
        );
    }
}

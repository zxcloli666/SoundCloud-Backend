use std::future::Future;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

use tracing::{debug, info, warn};
use url::Url;

use super::ytdlp::Exit;

const FALLBACK_TRIES: usize = 2;

struct Entry {
    url: String,
    russian: bool,
}

#[derive(Default)]
struct Loaded {
    seen: bool,
    stamp: Option<SystemTime>,
    entries: Vec<Entry>,
}

pub(crate) struct ProxyPool {
    path: Option<PathBuf>,
    loaded: Mutex<Loaded>,
}

impl ProxyPool {
    pub(crate) fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            loaded: Mutex::new(Loaded::default()),
        }
    }

    pub(crate) async fn routed<T, E, F, Fut>(&self, exit: Exit, attempt: F) -> Result<T, E>
    where
        F: Fn(Option<String>) -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let mut last = attempt(None).await;
        if last.is_ok() {
            return last;
        }
        for proxy in self.pick(exit, FALLBACK_TRIES) {
            debug!(proxy = endpoint(&proxy), "direct failed, trying a proxy");
            last = attempt(Some(proxy)).await;
            if last.is_ok() {
                break;
            }
        }
        last
    }

    fn pick(&self, exit: Exit, count: usize) -> Vec<String> {
        let mut candidates = self.matching(exit);
        let mut chosen = Vec::new();
        while chosen.len() < count && !candidates.is_empty() {
            let index = (uuid::Uuid::new_v4().as_u128() % candidates.len() as u128) as usize;
            chosen.push(candidates.swap_remove(index));
        }
        chosen
    }

    fn matching(&self, exit: Exit) -> Vec<String> {
        let Some(path) = &self.path else {
            return Vec::new();
        };
        let stamp = std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok();
        let mut loaded = self
            .loaded
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if !loaded.seen || loaded.stamp != stamp {
            loaded.entries = std::fs::read_to_string(path)
                .map(|text| parse(&text))
                .unwrap_or_default();
            loaded.stamp = stamp;
            loaded.seen = true;
            if loaded.entries.is_empty() {
                warn!(path = %path.display(), "video proxy list is missing or has no usable proxies");
            } else {
                info!(count = loaded.entries.len(), "video proxy list loaded");
            }
        }
        loaded
            .entries
            .iter()
            .filter(|entry| exit == Exit::Anywhere || entry.russian)
            .map(|entry| entry.url.clone())
            .collect()
    }
}

fn parse(text: &str) -> Vec<Entry> {
    text.lines().filter_map(entry).collect()
}

fn entry(line: &str) -> Option<Entry> {
    let line = line.trim();
    if line.starts_with('#') {
        return None;
    }
    let mut tokens = line.split_whitespace();
    let url = tokens.next()?;
    let tags: Vec<&str> = tokens
        .filter_map(|token| token.strip_prefix("tags="))
        .flat_map(|list| list.split(','))
        .collect();
    let socks = matches!(Url::parse(url).ok()?.scheme(), "socks5" | "socks5h");
    if !socks || !tags.contains(&"dc") || tags.contains(&"reserve") {
        return None;
    }
    Some(Entry {
        url: url.to_owned(),
        russian: tags.contains(&"geo-ru"),
    })
}

fn endpoint(proxy: &str) -> String {
    Url::parse(proxy)
        .ok()
        .and_then(|url| Some(format!("{}:{}", url.host_str()?, url.port()?)))
        .unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::io::Write;
    use std::time::Duration;

    use super::*;

    const SAMPLE: &str = "\
# rotates every 6 days
socks5://user1:pass1@10.0.0.1:1080  tags=dc
socks5h://user2:pass2@10.0.0.2:1080  tags=dc,geo-ru
socks5://user3:pass3@10.0.0.3:1080  tags=dc,reserve
socks5://user4:pass4@10.0.0.4:1080  tags=residential
http://user5:pass5@10.0.0.5:8080  tags=dc
socks5://user6:pass6@10.0.0.6:1080  tags=dc,geo-ru,reserve

not a url tags=dc
";

    fn file(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proxies.txt");
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }

    fn urls(pool: &ProxyPool, exit: Exit) -> HashSet<String> {
        pool.matching(exit).into_iter().collect()
    }

    #[test]
    fn only_socks_dc_lines_without_reserve_are_used() {
        let entries = parse(SAMPLE);

        let found: Vec<&str> = entries.iter().map(|entry| entry.url.as_str()).collect();
        assert_eq!(
            found,
            [
                "socks5://user1:pass1@10.0.0.1:1080",
                "socks5h://user2:pass2@10.0.0.2:1080"
            ]
        );
    }

    #[test]
    fn the_russian_exit_takes_only_geo_ru_lines() {
        let (_dir, path) = file(SAMPLE);
        let pool = ProxyPool::new(Some(path));

        assert_eq!(urls(&pool, Exit::Anywhere).len(), 2);
        assert_eq!(
            urls(&pool, Exit::Russia),
            HashSet::from(["socks5h://user2:pass2@10.0.0.2:1080".to_owned()])
        );
    }

    #[test]
    fn the_file_is_read_again_when_its_mtime_changes() {
        let (_dir, path) = file("socks5://a:b@10.0.0.1:1080 tags=dc\n");
        let pool = ProxyPool::new(Some(path.clone()));
        assert_eq!(urls(&pool, Exit::Anywhere).len(), 1);

        std::fs::write(&path, "socks5://a:b@10.0.0.9:1080 tags=dc\n").unwrap();
        let later = SystemTime::now() + Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert_eq!(
            urls(&pool, Exit::Anywhere),
            HashSet::from(["socks5://a:b@10.0.0.9:1080".to_owned()])
        );

        let mut same = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        same.write_all(b"socks5://a:b@10.0.0.8:1080 tags=dc\n")
            .unwrap();
        same.set_modified(later).unwrap();
        assert_eq!(urls(&pool, Exit::Anywhere).len(), 1);
    }

    #[test]
    fn a_missing_or_empty_file_gives_no_proxy_and_a_later_file_is_picked_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proxies.txt");
        let pool = ProxyPool::new(Some(path.clone()));
        assert!(pool.pick(Exit::Anywhere, 2).is_empty());

        std::fs::write(&path, "# nothing\n").unwrap();
        assert!(pool.pick(Exit::Anywhere, 2).is_empty());

        std::fs::write(&path, "socks5://a:b@10.0.0.1:1080 tags=dc\n").unwrap();
        assert_eq!(pool.pick(Exit::Anywhere, 2).len(), 1);
        assert!(ProxyPool::new(None).pick(Exit::Anywhere, 2).is_empty());
    }

    #[test]
    fn a_pick_is_distinct_and_capped_by_the_set() {
        let (_dir, path) = file(
            "socks5://a:b@10.0.0.1:1080 tags=dc\nsocks5://a:b@10.0.0.2:1080 tags=dc\nsocks5://a:b@10.0.0.3:1080 tags=dc\n",
        );
        let pool = ProxyPool::new(Some(path));

        for _ in 0..20 {
            let picked = pool.pick(Exit::Anywhere, 2);
            assert_eq!(picked.len(), 2);
            assert_ne!(picked[0], picked[1]);
        }
        assert_eq!(pool.pick(Exit::Anywhere, 9).len(), 3);
    }

    #[test]
    fn a_log_endpoint_never_carries_credentials() {
        assert_eq!(
            endpoint("socks5://user:secret@10.0.0.1:1080"),
            "10.0.0.1:1080"
        );
        assert_eq!(endpoint("garbage"), "unknown");
    }

    async fn run(
        pool: &ProxyPool,
        exit: Exit,
        works_on: usize,
    ) -> (Result<usize, String>, Vec<Option<String>>) {
        let calls = Mutex::new(Vec::new());
        let result = pool
            .routed(exit, |proxy| {
                let mut calls = calls.lock().unwrap();
                calls.push(proxy);
                let call = calls.len();
                async move {
                    if call == works_on {
                        Ok(call)
                    } else {
                        Err(format!("call {call} failed"))
                    }
                }
            })
            .await;
        (result, calls.into_inner().unwrap())
    }

    fn three() -> (tempfile::TempDir, ProxyPool) {
        let (dir, path) = file(
            "socks5://a:b@10.0.0.1:1080 tags=dc\nsocks5://a:b@10.0.0.2:1080 tags=dc\nsocks5://a:b@10.0.0.3:1080 tags=dc,geo-ru\n",
        );
        (dir, ProxyPool::new(Some(path)))
    }

    #[tokio::test]
    async fn a_direct_success_never_touches_a_proxy() {
        let (_dir, pool) = three();

        let (result, calls) = run(&pool, Exit::Anywhere, 1).await;

        assert_eq!(result, Ok(1));
        assert_eq!(calls, [None]);
    }

    #[tokio::test]
    async fn a_direct_failure_tries_one_proxy_then_stops_on_success() {
        let (_dir, pool) = three();

        let (result, calls) = run(&pool, Exit::Anywhere, 2).await;

        assert_eq!(result, Ok(2));
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], None);
        assert!(calls[1].is_some());
    }

    #[tokio::test]
    async fn two_distinct_proxies_then_the_last_error() {
        let (_dir, pool) = three();

        let (result, calls) = run(&pool, Exit::Anywhere, 99).await;

        assert_eq!(result, Err("call 3 failed".to_owned()));
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0], None);
        assert_ne!(calls[1], calls[2]);
    }

    #[tokio::test]
    async fn the_russian_exit_falls_back_only_through_geo_ru_proxies() {
        let (_dir, pool) = three();

        let (result, calls) = run(&pool, Exit::Russia, 99).await;

        assert_eq!(result, Err("call 2 failed".to_owned()));
        assert_eq!(calls, [None, Some("socks5://a:b@10.0.0.3:1080".to_owned())]);
    }

    #[tokio::test]
    async fn without_proxies_the_direct_error_stands() {
        let pool = ProxyPool::new(None);

        let (result, calls) = run(&pool, Exit::Anywhere, 99).await;

        assert_eq!(result, Err("call 1 failed".to_owned()));
        assert_eq!(calls, [None]);
    }
}

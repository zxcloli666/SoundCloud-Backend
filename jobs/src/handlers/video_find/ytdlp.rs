use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::sources::Failure;

const SEARCH_TIMEOUT: Duration = Duration::from_secs(60);
const SOCKET_TIMEOUT_S: &str = "20";
const RETRIES: &str = "2";
const SELECTOR: &str = "best[ext=mp4][height<=720]/best[ext=mp4]/best[height<=720]/best";
const WATCH: &str = "https://www.youtube.com/watch?v=";
const STDERR_TAIL: usize = 300;
const RATE_LIMIT: [&str; 5] = [
    "not a bot",
    "http error 429",
    "too many requests",
    "rate-limited",
    "rate limited",
];
const AUTH: [&str; 6] = [
    "cookies are no longer valid",
    "account has been terminated",
    "login required",
    "http error 401",
    "sign in to confirm your age",
    "please sign in",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Exit {
    Anywhere,
    Russia,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Found {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) duration_s: Option<f64>,
}

impl Found {
    pub(crate) fn url(&self) -> String {
        format!("{WATCH}{}", self.id)
    }
}

pub(crate) trait Tube: Send + Sync {
    fn search(
        &self,
        query: &str,
        results: usize,
        cookies: &str,
    ) -> impl Future<Output = Result<Vec<Found>, Failure>> + Send;

    fn download(
        &self,
        url: &str,
        stem: &Path,
        cookies: Option<&str>,
        exit: Exit,
    ) -> impl Future<Output = Result<PathBuf, Failure>> + Send;
}

pub(crate) struct YtDlp {
    binary: PathBuf,
    proxy: Option<String>,
    proxy_ru: Option<String>,
    work_dir: PathBuf,
    download_timeout: Duration,
    max_file_mib: u64,
}

impl YtDlp {
    pub(crate) fn new(config: &crate::config::VideoFindConfig) -> Self {
        Self {
            binary: config.ytdlp.clone(),
            proxy: config.proxy_url.clone(),
            proxy_ru: config
                .proxy_ru_url
                .clone()
                .or_else(|| config.proxy_url.clone()),
            work_dir: config.work_dir.clone(),
            download_timeout: config.download_timeout,
            max_file_mib: config.max_file_mib,
        }
    }

    fn proxy_for(&self, exit: Exit) -> Option<&str> {
        match exit {
            Exit::Anywhere => self.proxy.as_deref(),
            Exit::Russia => self.proxy_ru.as_deref(),
        }
    }

    async fn run(
        &self,
        args: &[String],
        cookies: Option<&str>,
        proxy: Option<&str>,
        timeout: Duration,
    ) -> Result<Vec<u8>, Failure> {
        let jar = CookieJar::write(&self.work_dir, cookies)
            .await
            .map_err(|error| Failure::Unavailable(format!("cookie file: {error}")))?;
        let mut command = Command::new(&self.binary);
        command
            .args([
                "--ignore-config",
                "--no-warnings",
                "--no-colors",
                "--js-runtimes",
                "deno",
                "--socket-timeout",
                SOCKET_TIMEOUT_S,
                "--retries",
                RETRIES,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(jar) = &jar {
            command.arg("--cookies").arg(jar.path());
        }
        if let Some(proxy) = proxy {
            command.args(["--proxy", proxy]);
        }
        command.args(args);
        let output = tokio::time::timeout(timeout, command.output())
            .await
            .map_err(|_| Failure::Unavailable("yt-dlp timed out".to_owned()))?
            .map_err(|error| Failure::Unavailable(format!("yt-dlp did not start: {error}")))?;
        if output.status.success() {
            return Ok(output.stdout);
        }
        Err(classify(&String::from_utf8_lossy(&output.stderr)))
    }

    async fn run_routed(
        &self,
        args: &[String],
        cookies: Option<&str>,
        exit: Exit,
        timeout: Duration,
    ) -> Result<Vec<u8>, Failure> {
        let direct = self.run(args, cookies, None, timeout).await;
        match (direct, self.proxy_for(exit)) {
            (Err(_), Some(proxy)) => self.run(args, cookies, Some(proxy), timeout).await,
            (other, _) => other,
        }
    }
}

impl Tube for YtDlp {
    async fn search(
        &self,
        query: &str,
        results: usize,
        cookies: &str,
    ) -> Result<Vec<Found>, Failure> {
        let args = [
            "--flat-playlist".to_owned(),
            "--dump-single-json".to_owned(),
            format!("ytsearch{results}:{query}"),
        ];
        let stdout = self
            .run_routed(&args, Some(cookies), Exit::Anywhere, SEARCH_TIMEOUT)
            .await?;
        Ok(parse_search(&stdout))
    }

    async fn download(
        &self,
        url: &str,
        stem: &Path,
        cookies: Option<&str>,
        exit: Exit,
    ) -> Result<PathBuf, Failure> {
        let args = [
            "--no-playlist".to_owned(),
            "--no-part".to_owned(),
            "--no-continue".to_owned(),
            "--max-filesize".to_owned(),
            format!("{}M", self.max_file_mib),
            "-f".to_owned(),
            SELECTOR.to_owned(),
            "-o".to_owned(),
            format!("{}.%(ext)s", stem.display()),
            url.to_owned(),
        ];
        self.run_routed(&args, cookies, exit, self.download_timeout)
            .await?;
        produced(stem).await
    }
}

async fn produced(stem: &Path) -> Result<PathBuf, Failure> {
    let missing = || Failure::Unavailable("yt-dlp produced no file".to_owned());
    let folder = stem.parent().ok_or_else(missing)?;
    let prefix = format!(
        "{}.",
        stem.file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(missing)?
    );
    let mut entries = tokio::fs::read_dir(folder).await.map_err(|_| missing())?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            return Ok(entry.path());
        }
    }
    Err(missing())
}

struct CookieJar {
    path: PathBuf,
}

impl CookieJar {
    async fn write(folder: &Path, text: Option<&str>) -> std::io::Result<Option<Self>> {
        let Some(text) = text.filter(|text| !text.is_empty()) else {
            return Ok(None);
        };
        tokio::fs::create_dir_all(folder).await?;
        let path = folder.join(format!("cookies-{}.txt", uuid::Uuid::new_v4()));
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .await?;
        file.write_all(text.as_bytes()).await?;
        file.flush().await?;
        Ok(Some(Self { path }))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for CookieJar {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(super) fn classify(stderr: &str) -> Failure {
    let lowered = stderr.to_lowercase();
    let tail = tail(stderr);
    if RATE_LIMIT.iter().any(|marker| lowered.contains(marker)) {
        Failure::Paused(format!("youtube: {tail}"))
    } else if AUTH.iter().any(|marker| lowered.contains(marker)) {
        Failure::Rejected(format!("youtube: {tail}"))
    } else {
        Failure::Unavailable(tail)
    }
}

fn tail(stderr: &str) -> String {
    let flat = stderr.split_whitespace().collect::<Vec<_>>().join(" ");
    let skip = flat.chars().count().saturating_sub(STDERR_TAIL);
    flat.chars().skip(skip).collect()
}

pub(super) fn parse_search(stdout: &[u8]) -> Vec<Found> {
    let Ok(reply) = serde_json::from_slice::<Value>(stdout) else {
        return Vec::new();
    };
    reply["entries"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let id = entry["id"].as_str().filter(|id| !id.is_empty())?;
                    Some(Found {
                        id: id.to_owned(),
                        title: entry["title"].as_str().unwrap_or_default().to_owned(),
                        duration_s: entry["duration"].as_f64(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bot_check_pauses_the_account_and_a_login_wall_kills_it() {
        assert!(matches!(
            classify("ERROR: Sign in to confirm you're not a bot"),
            Failure::Paused(_)
        ));
        assert!(matches!(
            classify("ERROR: The provided YouTube account cookies are no longer valid"),
            Failure::Rejected(_)
        ));
        assert!(matches!(
            classify("ERROR: Video unavailable"),
            Failure::Unavailable(_)
        ));
    }

    #[test]
    fn a_search_reply_becomes_ids_titles_and_durations() {
        let reply = br#"{"entries":[{"id":"abc","title":"Song","duration":201.0},{"id":"","title":"x"},{"id":"nod","title":"No length"}]}"#;

        let found = parse_search(reply);

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].url(), "https://www.youtube.com/watch?v=abc");
        assert_eq!(found[0].duration_s, Some(201.0));
        assert_eq!(found[1].duration_s, None);
        assert!(parse_search(b"not json").is_empty());
    }
}

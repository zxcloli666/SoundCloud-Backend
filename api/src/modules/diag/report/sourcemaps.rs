use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use backend_contracts::diag_report::{Frame, ReportError, SourcemapState};
use flate2::read::GzDecoder;
use futures::StreamExt;
use mini_moka::sync::Cache;
use sourcemap::SourceMap;
use uuid::Uuid;
use wreq::redirect::{Action, Attempt, Policy};

const KEPT_VERSIONS: usize = 5;
const MAX_ARCHIVE_BYTES: usize = 200 * 1024 * 1024;
const MAX_MAP_BYTES: u64 = 20 * 1024 * 1024;
const MAX_ENTRIES: usize = 2000;
const MAX_UNPACKED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_REDIRECTS: usize = 3;
const FRAMES_PER_ERROR: usize = 20;
const FRAMES_PER_REPORT: usize = 200;
const PARSED_MAPS_BYTES: u64 = 64 * 1024 * 1024;
const TOKEN_BYTES: u32 = 32;
const MISSING_VERSIONS: u64 = 1024;
const STALE_PARTIAL: Duration = Duration::from_secs(3600);
const DEADLINE: Duration = Duration::from_secs(5);
const MISSING_FOR: Duration = Duration::from_secs(600);
const RELEASES: &str = "https://github.com/zxcloli666/SoundCloud-Desktop/releases/download";
const GITHUB_HOSTS: [&str; 3] = [
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

pub struct Sourcemaps {
    dir: PathBuf,
    http: wreq::Client,
    missing: Cache<String, ()>,
    flights: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    parsed: Cache<(String, String), Arc<SourceMap>>,
}

enum Fetched {
    Ready,
    Missing,
}

struct Lookup {
    error: usize,
    frame: usize,
    file: String,
    line: u32,
    column: u32,
}

struct Resolved {
    error: usize,
    frame: usize,
    source: Option<String>,
    source_line: u32,
    source_column: u32,
    symbol: Option<String>,
}

impl Sourcemaps {
    pub fn new(dir: PathBuf) -> Self {
        let http = wreq::Client::builder()
            .redirect(Policy::custom(follow_within_github))
            .build()
            .expect("a plain http client builds");
        Self {
            dir,
            http,
            missing: Cache::builder()
                .max_capacity(MISSING_VERSIONS)
                .time_to_live(MISSING_FOR)
                .build(),
            flights: Mutex::new(HashMap::new()),
            parsed: Cache::builder()
                .max_capacity(PARSED_MAPS_BYTES)
                .weigher(|_, map: &Arc<SourceMap>| {
                    map.get_token_count().saturating_mul(TOKEN_BYTES)
                })
                .build(),
        }
    }

    pub async fn apply(&self, version: &str, errors: &mut [ReportError]) -> SourcemapState {
        if errors.iter().all(|error| error.frames.is_empty()) {
            return SourcemapState::NotNeeded;
        }
        if !valid_version(version) {
            return SourcemapState::Missing;
        }
        let lookups = lookups(errors);
        let fetched = tokio::time::timeout(DEADLINE, self.ensure(version)).await;
        match fetched {
            Ok(Ok(Fetched::Ready)) => {}
            Ok(Ok(Fetched::Missing)) => return SourcemapState::Missing,
            Ok(Err(_)) | Err(_) => return SourcemapState::Failed,
        }
        let directory = self.dir.join(version);
        let parsed = self.parsed.clone();
        let version = version.to_owned();
        let Ok(resolved) =
            tokio::task::spawn_blocking(move || resolve(&directory, &version, &parsed, lookups))
                .await
        else {
            return SourcemapState::Failed;
        };
        if resolved.is_empty() {
            return SourcemapState::Missing;
        }
        for found in resolved {
            let frame = &mut errors[found.error].frames[found.frame];
            frame.source = found.source;
            frame.source_line = Some(found.source_line);
            frame.source_column = Some(found.source_column);
            frame.symbol = found.symbol;
        }
        SourcemapState::Applied
    }

    async fn ensure(&self, version: &str) -> anyhow::Result<Fetched> {
        let directory = self.dir.join(version);
        if tokio::fs::try_exists(&directory).await? {
            return Ok(Fetched::Ready);
        }
        let flight = self
            .flights
            .lock()
            .expect("the flight lock is never poisoned")
            .entry(version.to_owned())
            .or_default()
            .clone();
        let _turn = flight.lock().await;
        let fetched = if tokio::fs::try_exists(&directory).await? {
            Ok(Fetched::Ready)
        } else {
            self.fetch(version).await
        };
        self.flights
            .lock()
            .expect("the flight lock is never poisoned")
            .remove(version);
        fetched
    }

    async fn fetch(&self, version: &str) -> anyhow::Result<Fetched> {
        if self.missing.contains_key(&version.to_owned()) {
            return Ok(Fetched::Missing);
        }
        let url = format!("{RELEASES}/{version}/web-sourcemaps.tar.gz");
        let Some(archive) = self.download(&url).await? else {
            self.missing.insert(version.to_owned(), ());
            return Ok(Fetched::Missing);
        };
        tokio::fs::create_dir_all(&self.dir).await?;
        let dir = self.dir.clone();
        let name = version.to_owned();
        tokio::task::spawn_blocking(move || unpack(&archive, &dir, &name)).await??;
        prune(&self.dir, version).await?;
        Ok(Fetched::Ready)
    }

    async fn download(&self, url: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let response = self.http.get(url).send().await?;
        if response.status().as_u16() == 404 {
            return Ok(None);
        }
        if !response.status().is_success() {
            anyhow::bail!("github answered {}", response.status());
        }
        let mut archive = Vec::new();
        let mut body = response.bytes_stream();
        while let Some(chunk) = body.next().await {
            archive.extend_from_slice(&chunk?);
            if archive.len() > MAX_ARCHIVE_BYTES {
                anyhow::bail!("sourcemap archive is too large");
            }
        }
        Ok(Some(archive))
    }
}

fn follow_within_github(attempt: Attempt) -> Action {
    let host = attempt.uri.host().unwrap_or_default();
    let secure = attempt.uri.scheme_str() == Some("https");
    if attempt.previous.len() > MAX_REDIRECTS {
        return attempt.error("too many redirects");
    }
    if !secure || !GITHUB_HOSTS.contains(&host) {
        return attempt.error("redirect leaves github");
    }
    attempt.follow()
}

fn lookups(errors: &[ReportError]) -> Vec<Lookup> {
    let mut lookups = Vec::new();
    for (error, entry) in errors.iter().enumerate() {
        for (frame, found) in entry.frames.iter().take(FRAMES_PER_ERROR).enumerate() {
            if lookups.len() == FRAMES_PER_REPORT {
                return lookups;
            }
            lookups.push(lookup(error, frame, found));
        }
    }
    lookups
}

fn lookup(error: usize, frame: usize, found: &Frame) -> Lookup {
    Lookup {
        error,
        frame,
        file: found.file.rsplit('/').next().unwrap_or_default().to_owned(),
        line: found.line,
        column: found.column,
    }
}

fn resolve(
    directory: &Path,
    version: &str,
    parsed: &Cache<(String, String), Arc<SourceMap>>,
    lookups: Vec<Lookup>,
) -> Vec<Resolved> {
    let mut maps: HashMap<String, Option<Arc<SourceMap>>> = HashMap::new();
    let mut resolved = Vec::new();
    for lookup in lookups {
        let map = maps
            .entry(lookup.file.clone())
            .or_insert_with(|| load(directory, version, &lookup.file, parsed));
        let Some(token) = map.as_ref().and_then(|map| {
            map.lookup_token(
                lookup.line.saturating_sub(1),
                lookup.column.saturating_sub(1),
            )
        }) else {
            continue;
        };
        resolved.push(Resolved {
            error: lookup.error,
            frame: lookup.frame,
            source: token.get_source().map(str::to_owned),
            source_line: token.get_src_line() + 1,
            source_column: token.get_src_col() + 1,
            symbol: token.get_name().map(str::to_owned),
        });
    }
    resolved
}

fn load(
    directory: &Path,
    version: &str,
    file: &str,
    parsed: &Cache<(String, String), Arc<SourceMap>>,
) -> Option<Arc<SourceMap>> {
    let key = (version.to_owned(), file.to_owned());
    if let Some(map) = parsed.get(&key) {
        return Some(map);
    }
    let path = directory.join(format!("{file}.map"));
    if std::fs::metadata(&path).ok()?.len() > MAX_MAP_BYTES {
        return None;
    }
    let map = Arc::new(SourceMap::from_slice(&std::fs::read(path).ok()?).ok()?);
    parsed.insert(key, map.clone());
    Some(map)
}

fn valid_version(version: &str) -> bool {
    version.starts_with(|c: char| c.is_ascii_digit())
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
        && !version.contains("..")
}

fn unpack(archive: &[u8], dir: &Path, version: &str) -> anyhow::Result<()> {
    let partial = dir.join(format!(".partial-{version}-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&partial)?;
    let unpacked = unpack_into(archive, &partial);
    if unpacked.is_err() {
        let _ = std::fs::remove_dir_all(&partial);
        return unpacked;
    }
    let renamed = std::fs::rename(&partial, dir.join(version));
    if renamed.is_err() {
        let _ = std::fs::remove_dir_all(&partial);
    }
    Ok(renamed?)
}

fn unpack_into(archive: &[u8], partial: &Path) -> anyhow::Result<()> {
    let mut total = 0;
    let mut tar = tar::Archive::new(GzDecoder::new(archive));
    for (index, entry) in tar.entries()?.enumerate() {
        if index == MAX_ENTRIES {
            anyhow::bail!("sourcemap archive has too many entries");
        }
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.ends_with(".map") || entry.header().size()? > MAX_MAP_BYTES {
            continue;
        }
        let mut content = Vec::new();
        entry
            .by_ref()
            .take(MAX_MAP_BYTES)
            .read_to_end(&mut content)?;
        total += content.len() as u64;
        if total > MAX_UNPACKED_BYTES {
            anyhow::bail!("sourcemap archive unpacks too large");
        }
        std::fs::write(partial.join(name), content)?;
    }
    Ok(())
}

async fn prune(dir: &Path, current: &str) -> anyhow::Result<()> {
    let mut versions: Vec<(SystemTime, PathBuf)> = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let metadata = entry.metadata().await?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !metadata.is_dir() {
            continue;
        }
        if valid_version(&name) {
            versions.push((metadata.modified()?, entry.path()));
        } else if name.starts_with(".partial-")
            && metadata.modified()?.elapsed().unwrap_or_default() > STALE_PARTIAL
        {
            tokio::fs::remove_dir_all(entry.path()).await?;
        }
    }
    versions.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    for (_, path) in versions.into_iter().skip(KEPT_VERSIONS) {
        if path.file_name().and_then(|name| name.to_str()) != Some(current) {
            tokio::fs::remove_dir_all(path).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

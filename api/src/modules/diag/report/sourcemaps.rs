use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use backend_contracts::diag_report::{ReportError, SourcemapState};
use flate2::read::GzDecoder;
use mini_moka::sync::Cache;
use sourcemap::SourceMap;

const KEPT_VERSIONS: usize = 5;
const MAX_ARCHIVE_BYTES: usize = 200 * 1024 * 1024;
const MAX_MAP_BYTES: u64 = 64 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(5);
const MISSING_FOR: Duration = Duration::from_secs(600);
const RELEASES: &str = "https://github.com/zxcloli666/SoundCloud-Desktop/releases/download";

pub struct Sourcemaps {
    dir: PathBuf,
    http: wreq::Client,
    missing: Cache<String, ()>,
}

enum Fetched {
    Ready,
    Missing,
}

impl Sourcemaps {
    pub fn new(dir: PathBuf) -> Self {
        let http = wreq::Client::builder()
            .build()
            .expect("a plain http client builds");
        Self {
            dir,
            http,
            missing: Cache::builder().time_to_live(MISSING_FOR).build(),
        }
    }

    pub async fn apply(&self, version: &str, errors: &mut [ReportError]) -> SourcemapState {
        let frames: Vec<_> = errors
            .iter_mut()
            .flat_map(|error| error.frames.iter_mut())
            .collect();
        if frames.is_empty() {
            return SourcemapState::NotNeeded;
        }
        if !valid_version(version) {
            return SourcemapState::Missing;
        }
        let fetched = tokio::time::timeout(DEADLINE, self.fetch(version)).await;
        match fetched {
            Ok(Ok(Fetched::Ready)) => {}
            Ok(Ok(Fetched::Missing)) => return SourcemapState::Missing,
            Ok(Err(_)) | Err(_) => return SourcemapState::Failed,
        }
        let directory = self.dir.join(version);
        let mut found = false;
        for frame in frames {
            let name = frame.file.rsplit('/').next().unwrap_or_default();
            let Ok(bytes) = tokio::fs::read(directory.join(format!("{name}.map"))).await else {
                continue;
            };
            let Ok(map) = SourceMap::from_slice(&bytes) else {
                continue;
            };
            let Some(token) =
                map.lookup_token(frame.line.saturating_sub(1), frame.column.saturating_sub(1))
            else {
                continue;
            };
            frame.source = token.get_source().map(str::to_owned);
            frame.source_line = Some(token.get_src_line() + 1);
            frame.source_column = Some(token.get_src_col() + 1);
            frame.symbol = token.get_name().map(str::to_owned);
            found = true;
        }
        if found {
            SourcemapState::Applied
        } else {
            SourcemapState::Missing
        }
    }

    async fn fetch(&self, version: &str) -> anyhow::Result<Fetched> {
        let directory = self.dir.join(version);
        if tokio::fs::try_exists(&directory).await? {
            return Ok(Fetched::Ready);
        }
        if self.missing.contains_key(&version.to_owned()) {
            return Ok(Fetched::Missing);
        }
        let url = format!("{RELEASES}/{version}/web-sourcemaps.tar.gz");
        let response = self.http.get(url).send().await?;
        if response.status().as_u16() == 404 {
            self.missing.insert(version.to_owned(), ());
            return Ok(Fetched::Missing);
        }
        if !response.status().is_success() {
            anyhow::bail!("github answered {}", response.status());
        }
        let archive = response.bytes().await?;
        if archive.len() > MAX_ARCHIVE_BYTES {
            anyhow::bail!("sourcemap archive is too large");
        }
        tokio::fs::create_dir_all(&self.dir).await?;
        let target = directory.clone();
        tokio::task::spawn_blocking(move || unpack(&archive, &target)).await??;
        prune(&self.dir, version).await?;
        Ok(Fetched::Ready)
    }
}

fn valid_version(version: &str) -> bool {
    version.starts_with(|c: char| c.is_ascii_digit())
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
        && !version.contains("..")
}

fn unpack(archive: &[u8], target: &Path) -> anyhow::Result<()> {
    let partial = target.with_extension("partial");
    let _ = std::fs::remove_dir_all(&partial);
    std::fs::create_dir_all(&partial)?;
    let mut tar = tar::Archive::new(GzDecoder::new(archive));
    for entry in tar.entries()? {
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
        std::fs::write(partial.join(name), content)?;
    }
    std::fs::rename(&partial, target)?;
    Ok(())
}

async fn prune(dir: &Path, current: &str) -> anyhow::Result<()> {
    let mut versions: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let metadata = entry.metadata().await?;
        if metadata.is_dir() && entry.path().extension().is_none() {
            versions.push((metadata.modified()?, entry.path()));
        }
    }
    versions.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in versions.into_iter().skip(KEPT_VERSIONS) {
        if path.file_name().and_then(|name| name.to_str()) != Some(current) {
            tokio::fs::remove_dir_all(path).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_versions_are_looked_up() {
        assert!(valid_version("8.4.25"));
        assert!(valid_version("8.5.0-beta.1"));
        assert!(!valid_version("../etc"));
        assert!(!valid_version("8/../../x"));
        assert!(!valid_version(""));
        assert!(!valid_version("v8"));
    }

    #[test]
    fn unpack_keeps_only_map_files() {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, body) in [("web/assets/index-A.js.map", "{}"), ("notes.txt", "x")] {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_cksum();
            builder.append_data(&mut header, name, body.as_bytes()).ok();
        }
        let tarball = builder.into_inner().expect("tar");
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gzip, &tarball).expect("gzip");
        let archive = gzip.finish().expect("gzip");
        let dir = std::env::temp_dir().join(format!("scd-maps-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).expect("dir");
        let target = dir.join("8.4.25");

        unpack(&archive, &target).expect("unpack");

        assert!(target.join("index-A.js.map").exists());
        assert!(!target.join("notes.txt").exists());
        std::fs::remove_dir_all(dir).ok();
    }
}

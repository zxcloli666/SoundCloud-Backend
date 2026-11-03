use std::path::{Path, PathBuf};
use std::time::Instant;

use backend_contracts::diag::ErrorKind;
use backend_contracts::diag_report::{ReportError, SourcemapState};
use sourcemap::SourceMapBuilder;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

use super::super::frames;
use super::*;

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("scd-maps-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&dir).expect("dir");
    dir
}

fn archive(files: &[(String, usize)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, size) in files {
        let body = vec![b'x'; *size];
        let mut header = tar::Header::new_gnu();
        header.set_size(*size as u64);
        header.set_cksum();
        builder
            .append_data(&mut header, name, body.as_slice())
            .expect("append");
    }
    gzip(builder.into_inner().expect("tar"))
}

fn gzip(tarball: Vec<u8>) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut encoder, &tarball).expect("gzip");
    encoder.finish().expect("gzip")
}

fn big_map(tokens: u32) -> Vec<u8> {
    let mut builder = SourceMapBuilder::new(Some("main.js"));
    for index in 0..tokens {
        builder.add(
            0,
            index * 3,
            index / 80,
            index % 80,
            Some("src/player.ts"),
            Some("play"),
            false,
        );
    }
    let mut bytes = Vec::new();
    builder
        .into_sourcemap()
        .to_writer(&mut bytes)
        .expect("map serializes");
    bytes
}

fn error_with(frames_in_stack: usize, file: &str) -> ReportError {
    let stack = format!("assets/{file}:1:5000\n").repeat(frames_in_stack);
    ReportError {
        at: "2026-10-29T10:00:00.000Z".to_owned(),
        kind: ErrorKind::Render,
        message: "x".to_owned(),
        count: 1,
        fingerprint: "0123456789abcdef".to_owned(),
        context: Default::default(),
        component_stack: None,
        frames: frames::parse(&stack),
        issue: None,
        trace: None,
    }
}

fn names(path: &Path) -> Vec<String> {
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

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
    let dir = scratch();
    let tarball = archive(&[
        ("web/assets/index-A.js.map".to_owned(), 2),
        ("notes.txt".to_owned(), 1),
    ]);

    unpack(&tarball, &dir, "8.4.25").expect("unpack");

    assert_eq!(names(&dir.join("8.4.25")), vec!["index-A.js.map"]);
    assert_eq!(names(&dir), vec!["8.4.25"]);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_map_over_twenty_megabytes_is_skipped() {
    let dir = scratch();
    let tarball = archive(&[
        ("a/huge-A.js.map".to_owned(), 20 * 1024 * 1024 + 1),
        ("a/fine-B.js.map".to_owned(), 10),
    ]);

    unpack(&tarball, &dir, "8.4.25").expect("unpack");

    assert_eq!(names(&dir.join("8.4.25")), vec!["fine-B.js.map"]);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn an_archive_with_too_many_entries_is_refused_and_leaves_nothing() {
    let dir = scratch();
    let files: Vec<_> = (0..=MAX_ENTRIES)
        .map(|index| (format!("a/{index}-A.js.map"), 1))
        .collect();

    let result = unpack(&archive(&files), &dir, "8.4.25");

    assert!(result.is_err());
    assert!(names(&dir).is_empty());
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn prune_keeps_five_versions_whatever_their_dots() {
    let dir = scratch();
    for patch in 0..12 {
        let version = format!("8.4.{patch}");
        unpack(
            &archive(&[("web/main-A.js.map".to_owned(), 1024)]),
            &dir,
            &version,
        )
        .expect("unpack");
        prune(&dir, &version).await.expect("prune");
    }

    assert_eq!(names(&dir).len(), KEPT_VERSIONS);
    assert!(dir.join("8.4.11").exists());
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn prune_removes_stale_partials_and_spares_fresh_ones() {
    let dir = scratch();
    let stale = dir.join(".partial-8.4.1-old");
    let fresh = dir.join(".partial-8.4.2-new");
    std::fs::create_dir_all(&stale).expect("dir");
    std::fs::create_dir_all(&fresh).expect("dir");
    let old = SystemTime::now() - 2 * STALE_PARTIAL;
    std::fs::File::open(&stale)
        .expect("open")
        .set_modified(old)
        .expect("mtime");

    prune(&dir, "8.4.3").await.expect("prune");

    assert!(!stale.exists());
    assert!(fresh.exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn two_versions_unpacking_at_once_never_break_each_other() {
    let dir = scratch();
    let big = archive(
        &(0..40)
            .map(|index| (format!("a/x{index}-A.js.map"), 256 * 1024))
            .collect::<Vec<_>>(),
    );
    let small = archive(&[("b/only-B.js.map".to_owned(), 10)]);

    for round in 0..30 {
        let first = format!("8.{round}.25");
        let second = format!("8.{round}.26");
        let (one, two) = std::thread::scope(|scope| {
            let one = scope.spawn(|| unpack(&big, &dir, &first));
            let two = scope.spawn(|| unpack(&small, &dir, &second));
            (one.join().expect("thread"), two.join().expect("thread"))
        });
        one.expect("first unpack");
        two.expect("second unpack");
        assert_eq!(names(&dir.join(&first)).len(), 40);
        assert_eq!(names(&dir.join(&second)), vec!["only-B.js.map"]);
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn tar_entries_cannot_leave_the_version_dir() {
    let dir = scratch();
    let cache = dir.join("cache");
    std::fs::create_dir_all(&cache).expect("dir");
    let mut builder = tar::Builder::new(Vec::new());
    let mut add = |name: &str, kind: tar::EntryType, link: Option<&str>, body: &[u8]| {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(kind);
        header.set_size(body.len() as u64);
        if let Some(link) = link {
            header.set_link_name(link).expect("link");
        }
        header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
        header.set_cksum();
        builder.append(&header, body).expect("append");
    };
    add(
        "../../escape-up.js.map",
        tar::EntryType::Regular,
        None,
        b"{}",
    );
    add(
        "/tmp/escape-abs.js.map",
        tar::EntryType::Regular,
        None,
        b"{}",
    );
    add(
        "link.js.map",
        tar::EntryType::Symlink,
        Some("/etc/passwd"),
        b"",
    );
    add(
        "hard.js.map",
        tar::EntryType::Link,
        Some("/etc/passwd"),
        b"",
    );
    add("ok.js.map", tar::EntryType::Regular, None, b"{}");

    unpack(&gzip(builder.into_inner().expect("tar")), &cache, "8.4.25").expect("unpack");

    assert!(!dir.join("escape-up.js.map").exists());
    assert!(!cache.join("escape-up.js.map").exists());
    for entry in std::fs::read_dir(cache.join("8.4.25"))
        .expect("dir")
        .flatten()
    {
        assert!(!entry.file_type().expect("type").is_symlink());
    }
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_report_of_thousands_of_frames_parses_the_map_once_and_resolves_at_most_two_hundred() {
    let dir = scratch();
    std::fs::create_dir_all(dir.join("8.4.25")).expect("dir");
    let map = big_map(750_000);
    assert!(
        map.len() > 3 * 1024 * 1024,
        "the map must be a few megabytes: {}",
        map.len()
    );
    std::fs::write(dir.join("8.4.25/main-A.js.map"), map).expect("map");
    let maps = Sourcemaps::new(dir.clone());
    let mut errors: Vec<_> = (0..50).map(|_| error_with(160, "main-A.js")).collect();
    let frames_total: usize = errors.iter().map(|error| error.frames.len()).sum();
    assert_eq!(frames_total, 8000);

    let started = Instant::now();
    let state = maps.apply("8.4.25", &mut errors).await;
    let elapsed = started.elapsed();

    assert_eq!(state, SourcemapState::Applied);
    assert!(elapsed.as_secs() < 3, "{elapsed:?}");
    let resolved: usize = errors
        .iter()
        .map(|error| {
            error
                .frames
                .iter()
                .filter(|frame| frame.source.is_some())
                .count()
        })
        .sum();
    assert_eq!(resolved, FRAMES_PER_REPORT);
    for error in &errors[..10] {
        let resolved = error
            .frames
            .iter()
            .filter(|frame| frame.source.is_some())
            .count();
        assert_eq!(resolved, FRAMES_PER_ERROR);
    }
    assert_eq!(errors[0].frames[0].source.as_deref(), Some("src/player.ts"));
    assert_eq!(errors[0].frames[0].symbol.as_deref(), Some("play"));
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn a_map_over_twenty_megabytes_on_disk_is_left_raw() {
    let dir = scratch();
    std::fs::create_dir_all(dir.join("8.4.25")).expect("dir");
    let file = std::fs::File::create(dir.join("8.4.25/main-A.js.map")).expect("map");
    file.set_len(MAX_MAP_BYTES + 1).expect("size");
    let maps = Sourcemaps::new(dir.clone());
    let mut errors = vec![error_with(3, "main-A.js")];

    let state = maps.apply("8.4.25", &mut errors).await;

    assert_eq!(state, SourcemapState::Missing);
    assert!(errors[0].frames.iter().all(|frame| frame.source.is_none()));
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn a_redirect_that_leaves_github_is_not_followed() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let answer = "HTTP/1.1 302 Found\r\nLocation: https://evil.example/web-sourcemaps.tar.gz\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream.write_all(answer.as_bytes()).await.ok();
        }
    });
    let maps = Sourcemaps::new(scratch());

    let result = maps.download(&format!("http://127.0.0.1:{port}/a")).await;

    assert!(result.is_err());
}

#[tokio::test]
#[ignore = "downloads a real GitHub release asset through its 302"]
async fn a_real_release_asset_downloads_through_the_github_redirect() {
    let maps = Sourcemaps::new(scratch());

    let body = maps
        .download(&format!("{RELEASES}/8.6.1/latest.json"))
        .await
        .expect("the download works")
        .expect("the asset exists");

    let text = String::from_utf8(body).expect("latest.json is text");
    assert!(text.contains("\"version\""), "{text}");
}

#[tokio::test]
#[ignore = "downloads a real GitHub release asset through its 302"]
async fn a_release_without_maps_is_reported_missing_not_failed() {
    let maps = Sourcemaps::new(scratch());
    let mut errors = vec![error_with(1, "main-A.js")];

    let state = maps.apply("8.6.1", &mut errors).await;

    assert_eq!(state, SourcemapState::Missing);
}

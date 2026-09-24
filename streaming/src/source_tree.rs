use std::fs;
use std::path::{Path, PathBuf};

pub fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn read(relative: &str) -> String {
    let path = crate_root().join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{} is unreadable: {err}", path.display()))
}

pub fn sources() -> Vec<(String, String)> {
    sources_under("src")
}

pub fn sources_under(relative: &str) -> Vec<(String, String)> {
    let root = crate_root().join(relative);
    let mut paths = Vec::new();
    collect(&root, &mut paths);
    assert!(
        !paths.is_empty(),
        "{} holds no Rust sources; a guard is reading the wrong directory",
        root.display()
    );
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let shown = path
                .strip_prefix(crate_root())
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            let body = fs::read_to_string(&path).expect("every source is readable");
            (shown, body)
        })
        .collect()
}

fn collect(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("the source directory is readable") {
        let path = entry.expect("the directory entry is readable").path();
        if path.is_dir() {
            collect(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

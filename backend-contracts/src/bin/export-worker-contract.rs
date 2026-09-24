use std::path::PathBuf;
use std::process::ExitCode;

use backend_contracts::worker_contract::{CONTRACT_PATH, render_worker_contract};

fn main() -> ExitCode {
    let target = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(CONTRACT_PATH));

    if let Some(directory) = target.parent()
        && let Err(error) = std::fs::create_dir_all(directory)
    {
        eprintln!("cannot create {}: {error}", directory.display());
        return ExitCode::FAILURE;
    }
    match std::fs::write(&target, render_worker_contract()) {
        Ok(()) => {
            println!("{}", target.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("cannot write {}: {error}", target.display());
            ExitCode::FAILURE
        }
    }
}

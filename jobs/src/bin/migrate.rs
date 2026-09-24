use std::process::ExitCode;
use std::str::FromStr;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    let scope = match (arguments.next(), arguments.next()) {
        (Some(scope), None) => match jobs::MigrationScope::from_str(&scope) {
            Ok(scope) => scope,
            Err(error) => {
                eprintln!("jobs migration scope is invalid: {error}");
                return ExitCode::FAILURE;
            }
        },
        _ => {
            eprintln!("usage: migrate <core|ops|all>");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = jobs::init_telemetry() {
        eprintln!("jobs migration telemetry initialization failed: {error}");
        return ExitCode::FAILURE;
    }

    match jobs::run_migrations(scope).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, "jobs migrations stopped with an error");
            ExitCode::FAILURE
        }
    }
}

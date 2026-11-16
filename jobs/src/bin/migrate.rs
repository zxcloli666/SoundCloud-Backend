use std::process::ExitCode;
use std::str::FromStr;

const USAGE: &str = "usage: migrate <core|ops|all> | migrate core --to <version>";

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    tls_common::init_crypto();
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let (scope, core_target) = match parse(&arguments) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = jobs::init_telemetry() {
        eprintln!("jobs migration telemetry initialization failed: {error}");
        return ExitCode::FAILURE;
    }

    match jobs::run_migrations(scope, core_target).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, "jobs migrations stopped with an error");
            ExitCode::FAILURE
        }
    }
}

fn parse(arguments: &[String]) -> Result<(jobs::MigrationScope, Option<i64>), String> {
    match arguments {
        [scope] => Ok((scope_of(scope)?, None)),
        [scope, flag, version] if flag == "--to" => {
            let scope = scope_of(scope)?;
            if scope != jobs::MigrationScope::Core {
                return Err(format!("--to applies to the core scope only; {USAGE}"));
            }
            let version = version
                .parse::<i64>()
                .map_err(|_| format!("--to needs a migration number, got {version}"))?;
            Ok((scope, Some(version)))
        }
        _ => Err(USAGE.to_owned()),
    }
}

fn scope_of(value: &str) -> Result<jobs::MigrationScope, String> {
    jobs::MigrationScope::from_str(value)
        .map_err(|error| format!("jobs migration scope is invalid: {error}"))
}

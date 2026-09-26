use std::process::ExitCode;

#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    tls_common::init_crypto();
    if let Err(error) = jobs::init_telemetry() {
        eprintln!("jobs telemetry initialization failed: {error}");
        return ExitCode::FAILURE;
    }

    match jobs::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, "jobs stopped with an error");
            ExitCode::FAILURE
        }
    }
}

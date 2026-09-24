use tracing_subscriber::EnvFilter;

pub fn init() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let filter = match EnvFilter::try_from_default_env() {
        Ok(filter) => filter,
        Err(_) => EnvFilter::new("info,jobs=debug,sqlx=warn,tower_http=info"),
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .try_init()
}

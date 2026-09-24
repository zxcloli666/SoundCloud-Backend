use sqlx::PgPool;

#[derive(Clone, Copy, Debug, Default)]
pub struct Observation(i64);

impl Observation {
    pub const UNVERIFIED: Self = Self(0);

    pub async fn begin(pool: &PgPool) -> Result<Self, sqlx::Error> {
        sqlx::query_file_scalar!("queries/begin_observation.sql")
            .fetch_one(pool)
            .await
            .map(Self)
    }

    pub fn sequence(self) -> i64 {
        self.0
    }
}

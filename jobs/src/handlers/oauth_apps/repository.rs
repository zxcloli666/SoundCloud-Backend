use sqlx::PgPool;

const EGRESS_LOCK: i64 = 0x5343_445F_4F41;
const BOOTSTRAP_LOCK: i64 = 0x5343_445F_4F42;

pub struct OAuthRefreshRepository {
    pool: PgPool,
}

mod runtime;

#[cfg(test)]
mod tests;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;

use deadpool_redis::{Config, Runtime};

use super::*;
use crate::common::admission::PublicAdmission;
use crate::config::{AdmissionCfg, AdmissionLimitCfg};
use crate::modules;

#[test]
fn the_old_paths_mount_beside_the_new_ones_without_taking_any_of_them() -> anyhow::Result<()> {
    let admission = admission()?;
    let mounted = catch_unwind(AssertUnwindSafe(|| {
        Router::<AppState>::new()
            .merge(modules::me::router())
            .merge(modules::auth::router(admission))
            .merge(router())
    }));
    assert!(
        mounted.is_ok(),
        "an old client path now overlaps a route of the me or auth module"
    );

    let doubled = catch_unwind(|| router().merge(router()));
    assert!(
        doubled.is_err(),
        "merging a path twice no longer panics, so the mount above proves nothing"
    );
    Ok(())
}

fn admission() -> anyhow::Result<Arc<PublicAdmission>> {
    let redis = Config::from_url("redis://127.0.0.1:1").create_pool(Some(Runtime::Tokio1))?;
    let limit = AdmissionLimitCfg {
        per_client: 1,
        global: 1,
    };
    Ok(PublicAdmission::new(
        redis,
        AdmissionCfg {
            window: Duration::from_secs(1),
            timeout: Duration::from_secs(1),
            max_in_flight: 1,
            login: limit,
            link_create: limit,
            resolve: limit,
        },
    ))
}

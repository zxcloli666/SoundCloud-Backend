#[path = "sync_tests/test_helpers.rs"]
mod test_helpers;

#[path = "sync_tests/hub_tests.rs"]
mod hub_tests;

#[path = "sync_tests/unit_tests.rs"]
mod unit_tests;

#[path = "sync_tests/assemble_tests.rs"]
mod assemble_tests;

#[path = "sync_tests/redis_tests.rs"]
mod redis_tests;

#[path = "sync_tests/live_tests.rs"]
mod live_tests;

#[path = "sync_tests/soak_tests.rs"]
mod soak_tests;

#[path = "sync_tests/sealed_tests.rs"]
mod sealed_tests;

#[path = "sync_tests/hold_tests.rs"]
mod hold_tests;

#[path = "sync_tests/wake_tests.rs"]
mod wake_tests;

#[path = "sync_tests/rev_tests.rs"]
mod rev_tests;

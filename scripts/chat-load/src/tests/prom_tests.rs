use super::{buckets, quantile};

const BEFORE: &str = r#"
# HELP api_pg_pool_wait_seconds wait
api_pg_pool_wait_seconds_bucket{pool="chat",le="0.001"} 10
api_pg_pool_wait_seconds_bucket{pool="chat",le="0.1"} 10
api_pg_pool_wait_seconds_bucket{pool="chat",le="0.5"} 10
api_pg_pool_wait_seconds_bucket{pool="chat",le="+Inf"} 10
api_pg_pool_wait_seconds_bucket{pool="core",le="0.001"} 5
api_pg_pool_wait_seconds_bucket{pool="core",le="0.1"} 5
api_pg_pool_wait_seconds_bucket{pool="core",le="0.5"} 5
api_pg_pool_wait_seconds_bucket{pool="core",le="+Inf"} 5
api_pg_pool_wait_seconds_sum 1
"#;

const AFTER: &str = r#"
api_pg_pool_wait_seconds_bucket{pool="chat",le="0.001"} 100
api_pg_pool_wait_seconds_bucket{pool="chat",le="0.1"} 105
api_pg_pool_wait_seconds_bucket{pool="chat",le="0.5"} 115
api_pg_pool_wait_seconds_bucket{pool="chat",le="+Inf"} 115
api_pg_pool_wait_seconds_bucket{pool="core",le="0.001"} 5
api_pg_pool_wait_seconds_bucket{pool="core",le="0.1"} 5
api_pg_pool_wait_seconds_bucket{pool="core",le="0.5"} 5
api_pg_pool_wait_seconds_bucket{pool="core",le="+Inf"} 5
"#;

#[test]
fn buckets_add_up_across_label_sets() {
    let found = buckets(BEFORE, "api_pg_pool_wait_seconds");
    assert_eq!(found.get("0.001"), Some(&15.0));
    assert_eq!(found.get("+Inf"), Some(&15.0));
    assert!(buckets(BEFORE, "api_other").is_empty());
}

#[test]
fn the_quantile_reads_only_the_window() {
    let before = buckets(BEFORE, "api_pg_pool_wait_seconds");
    let after = buckets(AFTER, "api_pg_pool_wait_seconds");
    let p50 = quantile(&before, &after, 0.5).expect("a p50");
    assert!(p50 <= 0.001, "{p50}");
    let p99 = quantile(&before, &after, 0.99).expect("a p99");
    assert!(p99 > 0.1 && p99 <= 0.5, "{p99}");
    assert_eq!(quantile(&after, &after, 0.99), None);
}

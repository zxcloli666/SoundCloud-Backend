use crate::modules::chat::sweeps::redis_info::{field, publish, rejected_writes};

const MEMORY: &str = "# Memory\r\nused_memory:104857600\r\nused_memory_human:100.00M\r\nused_memory_rss:120000000\r\nmaxmemory:536870912\r\nmaxmemory_human:512.00M\r\n";

const ERRORS: &str = "# Errorstats\r\nerrorstat_ERR:count=3\r\nerrorstat_OOM:count=17\r\n";

#[test]
fn a_field_is_matched_by_its_whole_name() {
    assert_eq!(field(MEMORY, "used_memory"), Some(104_857_600));
    assert_eq!(field(MEMORY, "maxmemory"), Some(536_870_912));
    assert_eq!(field(MEMORY, "used_memory_peak"), None);
    assert_eq!(field("maxmemory:0\r\n", "maxmemory"), Some(0));
}

#[test]
fn rejected_writes_are_the_oom_errors_and_zero_when_there_are_none() {
    assert_eq!(rejected_writes(ERRORS), 17);
    assert_eq!(
        rejected_writes("# Errorstats\r\nerrorstat_ERR:count=3\r\n"),
        0
    );
    assert_eq!(rejected_writes(""), 0);
}

#[test]
fn the_three_series_reach_the_scrape() {
    crate::metrics::init();
    publish(MEMORY, ERRORS);
    let body = crate::metrics::render_for_tests().unwrap_or_default();
    for line in [
        "chat_redis_rt_used_bytes 104857600",
        "chat_redis_rt_max_bytes 536870912",
        "chat_redis_rt_rejected_total 17",
    ] {
        assert!(body.contains(line), "{line} is missing from\n{body}");
    }
}

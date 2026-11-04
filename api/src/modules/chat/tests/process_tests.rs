use super::{cpu_seconds, rss_bytes};

const STATUS: &str = "Name:\tapi\nVmPeak:\t  900000 kB\nVmRSS:\t  123456 kB\nThreads:\t30\n";

const STAT: &str = "42 (api worker (1)) S 1 42 42 0 -1 4194560 1000 0 0 0 250 50 0 0 20 0 30 0 100 900000000 30000 18446744073709551615";

#[test]
fn rss_is_read_from_the_status_in_bytes() {
    assert_eq!(rss_bytes(STATUS), Some(123_456 * 1024));
    assert_eq!(rss_bytes("Name:\tapi\n"), None);
}

#[test]
fn cpu_is_user_plus_system_ticks_even_with_brackets_in_the_name() {
    assert_eq!(cpu_seconds(STAT), Some(3.0));
    assert_eq!(cpu_seconds("42 api"), None);
}

#[cfg(target_os = "linux")]
#[test]
fn this_process_reports_both() {
    crate::metrics::init();
    super::sample();
    let body = crate::metrics::render_for_tests().unwrap_or_default();
    assert!(body.contains("chat_process_rss_bytes "), "{body}");
    assert!(body.contains("process_cpu_seconds_total "), "{body}");
}

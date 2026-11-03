use std::fs;

const CLOCK_TICKS_PER_SECOND: f64 = 100.0;
const RSS_GAUGE: &str = "chat_process_rss_bytes";
const CPU_GAUGE: &str = "process_cpu_seconds_total";

pub fn sample() {
    if let Some(rss) = fs::read_to_string("/proc/self/status")
        .ok()
        .as_deref()
        .and_then(rss_bytes)
    {
        metrics::gauge!(RSS_GAUGE).set(rss as f64);
    }
    if let Some(cpu) = fs::read_to_string("/proc/self/stat")
        .ok()
        .as_deref()
        .and_then(cpu_seconds)
    {
        metrics::gauge!(CPU_GAUGE).set(cpu);
    }
}

pub fn rss_bytes(status: &str) -> Option<u64> {
    let line = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?;
    let kib: u64 = line.trim().strip_suffix("kB")?.trim().parse().ok()?;
    Some(kib * 1024)
}

pub fn cpu_seconds(stat: &str) -> Option<f64> {
    let after_name = &stat[stat.rfind(')')? + 1..];
    let mut fields = after_name.split_whitespace().skip(11);
    let user: f64 = fields.next()?.parse().ok()?;
    let system: f64 = fields.next()?.parse().ok()?;
    Some((user + system) / CLOCK_TICKS_PER_SECOND)
}

#[cfg(test)]
#[path = "../tests/process_tests.rs"]
mod tests;

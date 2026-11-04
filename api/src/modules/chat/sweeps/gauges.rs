use chrono::{DateTime, Utc};

pub fn backlog(sweep: &'static str, oldest: Option<DateTime<Utc>>) {
    let seconds = oldest.map_or(0.0, |at| {
        (Utc::now() - at).num_milliseconds().max(0) as f64 / 1_000.0
    });
    metrics::gauge!("chat_sweep_backlog", "sweep" => sweep).set(seconds);
}

pub fn done(sweep: &'static str, items: u64) {
    metrics::counter!("chat_sweep_items_total", "sweep" => sweep).increment(items);
}

pub fn failed(sweep: &'static str) {
    metrics::counter!("chat_sweep_failures_total", "sweep" => sweep).increment(1);
}

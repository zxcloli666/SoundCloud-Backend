pub fn note(path: &'static str, lane: &'static str, outcome: &'static str) {
    metrics::counter!("auth_x_total", "path" => path, "lane" => lane, "outcome" => outcome)
        .increment(1);
}

use std::collections::BTreeMap;

pub type Buckets = BTreeMap<String, f64>;

pub fn buckets(text: &str, metric: &str) -> Buckets {
    let prefix = format!("{metric}_bucket{{");
    let mut found = Buckets::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix(&prefix) else {
            continue;
        };
        let Some((labels, value)) = rest.rsplit_once('}') else {
            continue;
        };
        let Some(le) = label(labels, "le") else {
            continue;
        };
        let Some(count) = value
            .split_whitespace()
            .next()
            .and_then(|v| v.parse::<f64>().ok())
        else {
            continue;
        };
        *found.entry(le).or_insert(0.0) += count;
    }
    found
}

fn label(labels: &str, name: &str) -> Option<String> {
    let start = labels.find(&format!("{name}=\""))? + name.len() + 2;
    let end = labels[start..].find('"')? + start;
    Some(labels[start..end].to_owned())
}

fn bound(le: &str) -> f64 {
    match le {
        "+Inf" => f64::INFINITY,
        other => other.parse().unwrap_or(f64::INFINITY),
    }
}

pub fn quantile(before: &Buckets, after: &Buckets, q: f64) -> Option<f64> {
    let mut rows: Vec<(f64, f64)> = after
        .iter()
        .map(|(le, count)| (bound(le), count - before.get(le).copied().unwrap_or(0.0)))
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total = rows.last()?.1;
    if total <= 0.0 {
        return None;
    }
    let target = q * total;
    let mut previous = (0.0, 0.0);
    for (le, count) in rows {
        if count >= target {
            if le.is_infinite() {
                return Some(previous.0);
            }
            let width = count - previous.1;
            let inside = match width > 0.0 {
                true => (target - previous.1) / width,
                false => 1.0,
            };
            return Some(previous.0 + (le - previous.0) * inside);
        }
        previous = (le, count);
    }
    Some(previous.0)
}

#[cfg(test)]
#[path = "tests/prom_tests.rs"]
mod tests;

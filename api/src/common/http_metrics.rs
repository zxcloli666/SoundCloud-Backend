use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

const MAX_ROUTES: usize = 500;

#[derive(Default, Clone, serde::Serialize)]
pub struct RouteStat {
    pub count: u64,
    pub total_ms: u64,
    pub max_ms: u64,
    pub errors: u64,
}

pub struct HttpMetrics {
    started: Instant,
    routes: Mutex<HashMap<String, RouteStat>>,
}

impl Default for HttpMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpMetrics {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            routes: Mutex::new(HashMap::new()),
        }
    }

    pub fn record(&self, key: &str, ms: u64, status: u16) {
        let mut m = self.routes.lock().unwrap();
        if let Some(e) = m.get_mut(key) {
            e.count += 1;
            e.total_ms += ms;
            if ms > e.max_ms {
                e.max_ms = ms;
            }
            if status >= 500 {
                e.errors += 1;
            }
        } else if m.len() < MAX_ROUTES {
            m.insert(
                key.to_string(),
                RouteStat {
                    count: 1,
                    total_ms: ms,
                    max_ms: ms,
                    errors: (status >= 500) as u64,
                },
            );
        }
    }

    pub fn uptime_secs(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    pub fn snapshot(&self) -> Vec<(String, RouteStat)> {
        self.routes
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

pub const UNMATCHED_ROUTE: &str = "unmatched";

pub fn route_label(matched: Option<&axum::extract::MatchedPath>) -> String {
    match matched.map(axum::extract::MatchedPath::as_str) {
        Some(pattern) if !pattern.is_empty() => pattern.to_owned(),
        _ => UNMATCHED_ROUTE.to_owned(),
    }
}

pub fn method_label(method: &axum::http::Method) -> &'static str {
    match *method {
        axum::http::Method::GET => "GET",
        axum::http::Method::HEAD => "HEAD",
        axum::http::Method::POST => "POST",
        axum::http::Method::PUT => "PUT",
        axum::http::Method::PATCH => "PATCH",
        axum::http::Method::DELETE => "DELETE",
        axum::http::Method::OPTIONS => "OPTIONS",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_route_nobody_matched_collapses_into_one_label() {
        assert_eq!(route_label(None), UNMATCHED_ROUTE);
    }

    #[test]
    fn a_method_nobody_serves_collapses_into_one_label() {
        let invented = axum::http::Method::from_bytes(b"BREW").expect("a token is a valid method");
        assert_eq!(method_label(&invented), "other");
        assert_eq!(method_label(&axum::http::Method::GET), "GET");
        assert_eq!(method_label(&axum::http::Method::DELETE), "DELETE");
    }
}

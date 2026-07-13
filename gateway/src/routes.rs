use std::collections::HashMap;
use std::sync::Arc;

use hyper::Uri;

/// A resolved upstream target. Only `http` upstreams — the gateway terminates TLS
/// at the edge and talks cleartext to backends.
pub struct Upstream {
    pub scheme: &'static str,
    pub authority: String, // host:port, e.g. "seaweed-s3:8333"
}

/// Host → upstream map. Exact host match, with an optional `*`/`_` catch-all.
pub struct RouteTable {
    exact: HashMap<String, Arc<Upstream>>,
    default: Option<Arc<Upstream>>,
}

impl RouteTable {
    /// Parses the `GATEWAY_ROUTES` block: one `host -> http://upstream:port` per
    /// line, `#` comments and blank lines ignored. `*` or `_` as host = catch-all.
    pub fn from_env_str(s: &str) -> Result<Self, String> {
        let mut exact = HashMap::new();
        let mut default = None;
        for (i, raw) in s.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (host, target) = line
                .split_once("->")
                .ok_or_else(|| format!("route line {}: missing `->` in {:?}", i + 1, raw))?;
            let host = host.trim().to_ascii_lowercase();
            let up = Arc::new(
                Upstream::parse(target.trim())
                    .map_err(|e| format!("route line {}: {}", i + 1, e))?,
            );
            if host == "*" || host == "_" {
                default = Some(up);
            } else if !host.is_empty() {
                exact.insert(host, up);
            }
        }
        Ok(Self { exact, default })
    }

    pub fn is_empty(&self) -> bool {
        self.exact.is_empty() && self.default.is_none()
    }

    /// Concrete route hostnames — the domain list handed to ACME. The catch-all
    /// contributes nothing (it can't be issued a certificate).
    pub fn hostnames(&self) -> Vec<String> {
        let mut v: Vec<String> = self.exact.keys().cloned().collect();
        v.sort();
        v
    }

    pub fn lookup(&self, host: &str) -> Option<&Arc<Upstream>> {
        let bare = host.split(':').next().unwrap_or(host);
        match self.exact.get(bare) {
            Some(u) => Some(u),
            None => {
                let lower = bare.to_ascii_lowercase();
                self.exact.get(&lower).or(self.default.as_ref())
            }
        }
    }
}

impl Upstream {
    fn parse(s: &str) -> Result<Self, String> {
        let uri: Uri = s.parse().map_err(|e| format!("bad upstream {s:?}: {e}"))?;
        let scheme = match uri.scheme_str() {
            None | Some("http") => "http",
            Some(other) => {
                return Err(format!(
                    "upstream {s:?}: only http is supported (got {other}); the gateway terminates TLS at the edge"
                ))
            }
        };
        let authority = uri
            .authority()
            .ok_or_else(|| format!("upstream {s:?}: missing host:port"))?
            .as_str()
            .to_string();
        Ok(Self { scheme, authority })
    }
}

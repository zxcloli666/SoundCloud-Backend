use std::net::IpAddr;
use std::path::PathBuf;

pub struct TlsConfig {
    pub domains: Vec<String>,
    pub email: String,
    pub cache_dir: PathBuf,
    pub staging: bool,
    pub https_port: u16,
    pub http_port: u16,
    pub http_redirect: bool,
    pub proxy: ProxyProtocolConfig,
}

#[derive(Clone, Debug, Default)]
pub struct ProxyProtocolConfig {
    pub(crate) enabled: bool,
    pub(crate) trusted_cidrs: Vec<IpCidr>,
    pub(crate) trusted_hosts: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IpCidr {
    net: IpAddr,
    prefix: u8,
}

impl IpCidr {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let (ip_s, prefix) = match s.split_once('/') {
            Some((a, b)) => (a, b.trim().parse::<u8>().ok()?),
            None => (s, if s.contains(':') { 128 } else { 32 }),
        };
        let net: IpAddr = ip_s.trim().parse().ok()?;
        let max = if net.is_ipv6() { 128 } else { 32 };
        if prefix > max {
            return None;
        }
        Some(Self { net, prefix })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.net, ip) {
            (IpAddr::V4(n), IpAddr::V4(i)) => prefix_match(&n.octets(), &i.octets(), self.prefix),
            (IpAddr::V6(n), IpAddr::V6(i)) => prefix_match(&n.octets(), &i.octets(), self.prefix),
            _ => false,
        }
    }
}

fn prefix_match(a: &[u8], b: &[u8], prefix: u8) -> bool {
    let full = (prefix / 8) as usize;
    if a[..full] != b[..full] {
        return false;
    }
    let rem = prefix % 8;
    if rem == 0 {
        return true;
    }
    let mask = 0xFFu8 << (8 - rem);
    (a[full] & mask) == (b[full] & mask)
}

impl TlsConfig {
    pub fn from_env() -> Option<Self> {
        if !env_bool("TLS_ENABLED", false) {
            return None;
        }

        let domains = parse_csv(&std::env::var("DOMAINS").unwrap_or_default());
        if domains.is_empty() {
            panic!("TLS_ENABLED=true but DOMAINS is empty (expected comma-separated domain list)");
        }

        let email = std::env::var("ACME_EMAIL").unwrap_or_else(|_| format!("admin@{}", domains[0]));
        let cache_dir = PathBuf::from(
            std::env::var("ACME_CACHE_DIR").unwrap_or_else(|_| "/var/cache/acme".to_string()),
        );

        Some(Self {
            domains,
            email,
            cache_dir,
            staging: env_bool("ACME_STAGING", false),
            https_port: env_u16("TLS_HTTPS_PORT", 443),
            http_port: env_u16("TLS_HTTP_PORT", 80),
            http_redirect: env_bool("TLS_HTTP_REDIRECT", true),
            proxy: ProxyProtocolConfig::from_env(),
        })
    }
}

impl ProxyProtocolConfig {
    pub fn from_env() -> Self {
        let enabled = env_bool("TLS_PROXY_PROTOCOL", false);
        let trusted_cidrs =
            parse_csv(&std::env::var("TLS_PROXY_TRUSTED_CIDRS").unwrap_or_default())
                .into_iter()
                .map(|value| {
                    IpCidr::parse(&value).unwrap_or_else(|| {
                        panic!("TLS_PROXY_TRUSTED_CIDRS contains invalid CIDR: {value}")
                    })
                })
                .collect::<Vec<_>>();
        let trusted_hosts =
            parse_csv(&std::env::var("TLS_PROXY_TRUSTED_HOSTS").unwrap_or_default());
        if enabled && trusted_cidrs.is_empty() && trusted_hosts.is_empty() {
            panic!(
                "TLS_PROXY_PROTOCOL=true requires TLS_PROXY_TRUSTED_CIDRS or TLS_PROXY_TRUSTED_HOSTS"
            );
        }
        Self {
            enabled,
            trusted_cidrs,
            trusted_hosts,
        }
    }

    pub fn disabled() -> Self {
        Self::default()
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(default)
}

fn env_u16(key: &str, default: u16) -> u16 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn parse_csv(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

use std::net::{IpAddr, SocketAddr};
use std::sync::LazyLock;

use regex::{Captures, Regex};

const ROLE_WORDS: [&str; 5] = ["relay", "user", "public", "backup", "service"];
const INTERNAL_SERVICES: &str = "backend|streaming|storage|haproxy|jaeger|postgres|pgbouncer|redis|nats|qdrant|vectors|minio|call-control|call|worker|jobs|api|pay|images|s3";

fn pattern(source: &str) -> Regex {
    Regex::new(source).expect("the scrub pattern compiles")
}

static QUERY: LazyLock<Regex> = LazyLock::new(|| pattern(r#"\?[^\s"'<>)]+"#));
static SECRET_PAIR: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"(?i)\b([\w-]*(?:token|secret|passw|sign|key|auth|session|client_id|credential|cookie)[\w-]*)=\S+",
    )
});
static JWT: LazyLock<Regex> = LazyLock::new(|| pattern(r"\beyJ[\w-]*\.[\w-]+(?:\.[\w-]*)?"));
static UUID: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b")
});
static IPV4: LazyLock<Regex> = LazyLock::new(|| pattern(r"\b\d{1,3}(?:\.\d{1,3}){3}\b"));
static IPV6_CANDIDATE: LazyLock<Regex> = LazyLock::new(|| pattern(r"[0-9A-Fa-f:.\[\]]{3,}"));
static INTERNAL_HOST: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"(?i)\bsc-[a-z0-9-]+|[a-z0-9.-]*(?:scdinternal\.site|scnative\.(?:dev|space))\b|[a-z0-9.-]*\.internal[a-z0-9.-]*|\b[rp]\d+\.(?:relay|pro)[a-z0-9.-]*",
    )
});
static SERVICE_PORT: LazyLock<Regex> =
    LazyLock::new(|| pattern(&format!(r"(?i)\b(?:{INTERNAL_SERVICES}):\d{{2,5}}\b")));
static LONG_HEX: LazyLock<Regex> = LazyLock::new(|| pattern(r"(?i)[0-9a-f]{16,}"));
static SHORT_HEX: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"(?i)(^|[^0-9a-z])[0-9a-f]{12}([^0-9a-z]|$)"));
static LONG_TOKEN: LazyLock<Regex> = LazyLock::new(|| pattern(r"[A-Za-z0-9_-]{21,}"));
static EMAIL: LazyLock<Regex> = LazyLock::new(|| pattern(r"\S+@\S+"));
static BARE_SOURCE: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"\b(?:relay_track|oauth|anon|cookies|restricted|cdn_proxy|cdn_redirect|live_hq)\b")
});
static SOURCE_NAME: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"\b(source|upstream) ([A-Za-z_][\w-]*)"));

pub fn scrub(text: &str) -> String {
    let text = QUERY.replace_all(text, "");
    let text = SECRET_PAIR.replace_all(&text, "$1=[hidden]");
    let text = JWT.replace_all(&text, "[token]");
    let text = UUID.replace_all(&text, "[id]");
    let text = IPV4.replace_all(&text, "[ip]");
    let text = IPV6_CANDIDATE.replace_all(&text, |found: &Captures| {
        let candidate = &found[0];
        if is_ipv6(candidate) {
            "[ip]".to_owned()
        } else {
            candidate.to_owned()
        }
    });
    let text = INTERNAL_HOST.replace_all(&text, "[host]");
    let text = SERVICE_PORT.replace_all(&text, "[host]");
    let text = LONG_HEX.replace_all(&text, "[id]");
    let text = SHORT_HEX.replace_all(&text, "${1}[id]${2}");
    let text = LONG_TOKEN.replace_all(&text, |found: &Captures| {
        let token = &found[0];
        let mixed = token.bytes().any(|b| b.is_ascii_digit())
            && token.bytes().any(|b| b.is_ascii_alphabetic());
        if mixed {
            "[token]".to_owned()
        } else {
            token.to_owned()
        }
    });
    let text = EMAIL.replace_all(&text, "[email]");
    let text = BARE_SOURCE.replace_all(&text, |found: &Captures| source_role(&found[0]));
    SOURCE_NAME
        .replace_all(&text, |found: &Captures| {
            let name = &found[2];
            if ROLE_WORDS.contains(&name) {
                return found[0].to_owned();
            }
            match &found[1] {
                "source" => format!("source {}", source_role(name)),
                _ => "upstream service".to_owned(),
            }
        })
        .into_owned()
}

fn is_ipv6(candidate: &str) -> bool {
    let without_tail = candidate.trim_end_matches(['.', ':']);
    let without_lead = candidate
        .strip_prefix(':')
        .filter(|rest| !rest.starts_with(':'));
    [Some(candidate), Some(without_tail), without_lead]
        .into_iter()
        .flatten()
        .any(parses_as_ipv6)
}

fn parses_as_ipv6(text: &str) -> bool {
    let bare = text.trim_start_matches('[').split(']').next().unwrap_or("");
    bare.parse::<IpAddr>()
        .is_ok_and(|address| address.is_ipv6())
        || text
            .parse::<SocketAddr>()
            .is_ok_and(|address| address.is_ipv6())
}

pub fn source_role(name: &str) -> &'static str {
    match name {
        "relay_track" => "relay",
        "oauth" => "user session",
        "anon" => "public source",
        _ => "backup source",
    }
}

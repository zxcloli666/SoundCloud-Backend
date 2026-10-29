use axum::http::{HeaderMap, HeaderValue};

use super::*;

fn trusted() -> Vec<IpNet> {
    [
        "10.89.0.0/24",
        "2.27.22.81/32",
        "2.26.99.107/32",
        "177.3.213.61/32",
        "37.230.228.128/32",
    ]
    .iter()
    .map(|net| net.parse().expect("net"))
    .collect()
}

fn lines(values: &[&[u8]]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for value in values {
        headers.append(
            "x-forwarded-for",
            HeaderValue::from_bytes(value).expect("value"),
        );
    }
    headers
}

fn ip(text: &str) -> IpAddr {
    text.parse().expect("ip")
}

#[test]
fn the_production_proxy_chains_resolve_to_the_real_client() {
    let haproxy = ip("10.89.0.5");
    let cases: Vec<(&str, HeaderMap, &str)> = vec![
        (
            "direct, client sends its own XFF",
            lines(&[b"6.6.6.6, 7.7.7.7", b"198.51.100.20"]),
            "198.51.100.20",
        ),
        (
            "direct, client sends non-ASCII XFF first",
            lines(&[b"6.6.6.6 \xff", b"198.51.100.20"]),
            "198.51.100.20",
        ),
        (
            "relay r2 replaced XFF",
            lines(&[b"198.51.100.21", b"37.230.228.128"]),
            "198.51.100.21",
        ),
        (
            "r2 via call1 backup",
            lines(&[b"198.51.100.22", b"177.3.213.61"]),
            "198.51.100.22",
        ),
        (
            "relay-pro p1 copies client XFF then appends owner",
            lines(&[b"6.6.6.6", b"198.51.100.23", b"177.3.213.61"]),
            "198.51.100.23",
        ),
        (
            "ipv4-mapped hops",
            lines(&[b"::ffff:198.51.100.24", b"::ffff:37.230.228.128"]),
            "198.51.100.24",
        ),
        (
            "every hop trusted",
            lines(&[b"10.89.0.9", b"37.230.228.128"]),
            "10.89.0.5",
        ),
        (
            "garbage right of a trusted hop",
            lines(&[b"6.6.6.6", b"nonsense", b"37.230.228.128"]),
            "10.89.0.5",
        ),
    ];
    for (name, headers, want) in cases {
        assert_eq!(
            client_address(haproxy, &headers, &trusted()),
            ip(want),
            "{name}"
        );
    }
}

#[test]
fn ipv6_clients_share_one_bucket_per_64() {
    assert_eq!(
        client_identity(ip("2001:db8:1:2::1")),
        client_identity(ip("2001:db8:1:2:ffff::9"))
    );
    assert_ne!(
        client_identity(ip("2001:db8:1:2::1")),
        client_identity(ip("2001:db8:1:3::1"))
    );
}

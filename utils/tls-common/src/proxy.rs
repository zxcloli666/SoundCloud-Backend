use std::io;
use std::net::{IpAddr, SocketAddr};

use tokio::io::{AsyncRead, AsyncReadExt};

pub(crate) async fn read_proxy_v1<S: AsyncRead + Unpin>(stream: &mut S) -> io::Result<SocketAddr> {
    let mut buf = [0u8; 256];
    let mut pos = 0;
    loop {
        if pos >= buf.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "PROXY v1 line too long",
            ));
        }
        let n = stream.read(&mut buf[pos..pos + 1]).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "EOF before PROXY v1 CRLF",
            ));
        }
        pos += 1;
        if pos >= 2 && &buf[pos - 2..pos] == b"\r\n" {
            break;
        }
    }
    let line = std::str::from_utf8(&buf[..pos - 2])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "PROXY v1 not utf8"))?;

    let mut parts = line.split_ascii_whitespace();
    if parts.next() != Some("PROXY") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing PROXY signature",
        ));
    }
    let protocol = parts.next().unwrap_or("");
    let src_ip = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "PROXY v1 missing src_ip"))?;
    let dst_ip = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "PROXY v1 missing dst_ip"))?;
    let src_port = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "PROXY v1 missing src_port"))?;
    let dst_port = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "PROXY v1 missing dst_port"))?;
    if parts.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "PROXY v1 has trailing fields",
        ));
    }

    let ip: IpAddr = src_ip
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad src_ip"))?;
    let destination: IpAddr = dst_ip
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad dst_ip"))?;
    let port: u16 = src_port
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad src_port"))?;
    dst_port
        .parse::<u16>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad dst_port"))?;
    let valid_family = matches!(
        (protocol, ip, destination),
        ("TCP4", IpAddr::V4(_), IpAddr::V4(_)) | ("TCP6", IpAddr::V6(_), IpAddr::V6(_))
    );
    if !valid_family {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "PROXY v1 protocol does not match address family",
        ));
    }
    Ok(SocketAddr::new(ip, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parses_tcp4_and_tcp6() {
        let ipv4 = read_proxy_v1(&mut &b"PROXY TCP4 192.0.2.1 198.51.100.2 1234 443\r\n"[..])
            .await
            .unwrap();
        let ipv6 = read_proxy_v1(&mut &b"PROXY TCP6 2001:db8::1 2001:db8::2 4321 443\r\n"[..])
            .await
            .unwrap();

        assert_eq!(ipv4, "192.0.2.1:1234".parse().unwrap());
        assert_eq!(ipv6, "[2001:db8::1]:4321".parse().unwrap());
    }

    #[tokio::test]
    async fn rejects_mismatched_family_and_trailing_fields() {
        let mismatched =
            read_proxy_v1(&mut &b"PROXY TCP4 2001:db8::1 2001:db8::2 1 2\r\n"[..]).await;
        let trailing =
            read_proxy_v1(&mut &b"PROXY TCP4 192.0.2.1 198.51.100.2 1 2 extra\r\n"[..]).await;

        assert!(mismatched.is_err());
        assert!(trailing.is_err());
    }
}

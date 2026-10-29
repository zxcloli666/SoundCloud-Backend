import socket, ssl, sys

host, port, certs = sys.argv[1], int(sys.argv[2]), sys.argv[3]
PREFACE = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n" + b"\x00\x00\x00\x04\x00\x00\x00\x00\x00"


def attempt(name, cert=None, key=None, version=None, plain=False, verify_name=None):
    try:
        raw = socket.create_connection((host, port), timeout=5)
        if plain:
            raw.sendall(PREFACE)
            data = raw.recv(64)
            print(f"{name}: answered {data[:16]!r}" if data else f"{name}: closed with no answer")
            return
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        ctx.load_verify_locations(f"{certs}/ca.pem")
        ctx.check_hostname = True
        ctx.set_alpn_protocols(["h2"])
        if version:
            ctx.minimum_version = version
            ctx.maximum_version = version
        if cert:
            ctx.load_cert_chain(cert, key)
        conn = ctx.wrap_socket(raw, server_hostname=verify_name or host)
        conn.sendall(PREFACE)
        data = conn.recv(64)
        print(f"{name}: handshake ok ({conn.version()}, alpn {conn.selected_alpn_protocol()}), server sent {len(data)} bytes")
    except Exception as error:
        print(f"{name}: refused: {type(error).__name__}: {error}")


attempt("plain TCP, no TLS", plain=True)
attempt("TLS, no client cert")
attempt("TLS, client cert from another CA", f"{certs}/evil.crt", f"{certs}/evil.key")
attempt("TLS 1.1, good client cert", f"{certs}/a.crt", f"{certs}/a.key", ssl.TLSVersion.TLSv1_1)
attempt("TLS 1.2, good client cert", f"{certs}/a.crt", f"{certs}/a.key", ssl.TLSVersion.TLSv1_2)
attempt("TLS 1.3, good client cert", f"{certs}/a.crt", f"{certs}/a.key", ssl.TLSVersion.TLSv1_3)

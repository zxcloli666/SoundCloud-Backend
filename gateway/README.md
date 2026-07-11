# gateway

TLS-terminating reverse proxy in Rust. Replaces `https-portal`/nginx at the docker
edge. Auto-issues and renews Let's Encrypt certs (TLS-ALPN-01, via `tls-common`),
reverse-proxies by `Host` to cleartext upstreams with connection pooling, and shards
`accept()` across `SO_REUSEPORT` sockets (haproxy's `shards by-thread`, per worker).

Image: `ghcr.io/<owner>/soundcloud-backend/gateway` (built by `.github/workflows/auto-build.yml`).

## Data path

```
client :80/:443
      │  :443 → rustls termination (ACME cert, ALPN h2/http1.1)
      │  :80  → 301 https (default) or cleartext proxy
      ▼
  Host match → upstream (http://name:port)  ── pooled keep-alive ──▶ backend
```

- Streams request/response bodies (no buffering; unlimited body size).
- WebSocket / HTTP `Upgrade` tunnelled bidirectionally.
- `Host` forwarded verbatim (S3 SigV4 hashes it); adds `X-Forwarded-For/-Proto/-Host`, `X-Real-IP`.
- Certs via TLS-ALPN-01 on **:443** — issuance needs no `:80`, so HTTP can be turned off entirely.

## Config (env)

| var | default | meaning |
|-----|---------|---------|
| `GATEWAY_ROUTES` | — (required) | `host -> http://upstream:port` per line; `#` comments; `*`/`_` = catch-all |
| `HTTP_ENABLED` | `true` | bind `:80` |
| `HTTPS_ENABLED` | `true` | bind `:443` + ACME |
| `HTTP_MODE` | `redirect` | `:80` behaviour: `redirect` → 301 https, or `proxy` |
| `HTTP_PORT` / `HTTPS_PORT` | `80` / `443` | listen ports |
| `ACME_EMAIL` | `admin@<first-host>` | Let's Encrypt contact |
| `ACME_CACHE_DIR` | `/var/cache/acme` | cert cache (persist this volume) |
| `ACME_STAGING` | `false` | LE staging directory (testing) |
| `GATEWAY_SHARDS` | #cores | `SO_REUSEPORT` listeners per port |
| `WORKER_THREADS` | #cores | tokio worker threads |
| `POOL_MAX_IDLE_PER_HOST` | `4096` | pooled keep-alive conns per upstream |
| `POOL_IDLE_TIMEOUT_SECS` | `90` | idle pool eviction |
| `UPSTREAM_CONNECT_TIMEOUT_MS` | `5000` | upstream connect timeout |
| `SHUTDOWN_GRACE_SECS` | `10` | drain window on SIGTERM |
| `RUST_LOG` | `info` | log filter |

Only `http` upstreams — the gateway is the TLS edge; backends stay cleartext on the docker net.

## Docker

Default — owns `:80` + `:443` (like `https-portal` on s3-host / main-host now):

```yaml
  gateway:
    image: ghcr.io/<owner>/soundcloud-backend/gateway:latest
    restart: always
    ports: ["80:80", "443:443"]
    ulimits: { nofile: { soft: 300047, hard: 300047 } }
    sysctls: { net.ipv4.ip_local_port_range: "10000 65535" }
    environment:
      GATEWAY_ROUTES: |
        s3.scdinternal.site -> http://seaweed-s3:8333
    volumes: [acme:/var/cache/acme]
    healthcheck:
      test: ["CMD", "curl", "-fsS", "-o", "/dev/null", "http://127.0.0.1:80/"]
      interval: 30s
      timeout: 10s
      retries: 3
volumes: { acme: {} }
```

S3 wish — `:80` straight to SeaweedFS, only `:443` through us (certs still issue on 443):

```yaml
  seaweed-s3:
    ports: ["80:8333"]           # cleartext S3 direct on the host :80
  gateway:
    image: ghcr.io/<owner>/soundcloud-backend/gateway:latest
    restart: always
    ports: ["443:443"]           # gateway owns only :443
    environment:
      HTTP_ENABLED: "false"      # certs come via TLS-ALPN-01 on :443
      GATEWAY_ROUTES: |
        s3.scdinternal.site -> http://seaweed-s3:8333
    volumes: [acme:/var/cache/acme]
```

`build-context` for local builds: `docker build --build-context tls-common=../utils/tls-common ./gateway`.

## Highload

Ephemeral-port exhaustion (the ~28–64k `(src,dst)` limit to a single upstream) is avoided by
keep-alive pooling — connections are reused, not reopened per request. Widen the range and let
`TIME_WAIT` sockets be reused on the host:

```
net.ipv4.ip_local_port_range = 10000 65535
net.ipv4.tcp_tw_reuse = 1
```

Raise `POOL_MAX_IDLE_PER_HOST` for very high concurrency to one backend; add source IPs only if a
single upstream genuinely needs >50k concurrent connections.

DNS: point the domain at the host **before** first start, or Let's Encrypt rate-limits the retries.

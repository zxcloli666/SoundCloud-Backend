# chat-load

Load harness for chat-api. Every synthetic device holds its own connection, seals and signs every
request like the desktop (`utils/chat-wire`), parks or short-syncs, sends 1 message per 5 min, types
1 per 2 min and posts read marks on receive. Messages are real ciphertext under the chat key the
device fetched; receivers decrypt them and measure delivery time.

## Server side

- `CHAT_LOADTEST=on` on chat-api enables `POST /chat-admin/loadtest/seed` and `/purge` (token file
  as for every `/chat-admin/*` route). Off: both answer 404 and synthetic sessions never resolve.
- `seed {users, devices_per_user, groups}` creates users `lt:<n>` (numbering continues after the
  last seed), their devices (history unlocked) and chats: users are split into `groups` chats of
  2 to 50, small ones more likely, a chat of 2 is a DM. Sessions live in `chat:lt:sess:*` for 2 h.
  The answer is the layout: ids, sessions and device keys. Keep it private (`chmod 600`).
- `purge` drops the devices and their sessions, marks every `lt:` chat deleted and runs the normal
  sweep purge on them for up to 20 s, removes `lt:` members from real chats, blocks, reports, users
  and `chat:v|d|lim` keys. Call it until `pending` is 0 (`chat-load purge` does).
- Limits: 50 000 users and 50 000 devices per seed, 1 to 10 devices per user.

## Build

```sh
cargo build --release --locked
```

Linux build without installing anything on the host:

```sh
podman run --rm -v "$PWD/../..":/src -w /src mirror.gcr.io/library/rust:1-bookworm \
  cargo build --release --locked --manifest-path scripts/chat-load/Cargo.toml
```

## Commands

```sh
chat-load seed --admin http://chat-api:3000 --token-file token --users 20000 --groups 900 --out layout.json
chat-load run --layout layout.json [flags]
chat-load purge --admin http://chat-api:3000 --token-file token
chat-load fake-pro --bind 0.0.0.0:8080
```

`run` exits 0 on PASS, 3 on FAIL or abort. Main flags (defaults are the night plan):

| Flag | Default | Meaning |
|---|---|---|
| `--mix` | `direct=25,r1=25,r2=40,p1=10` | share of devices per route |
| `--relay-cap` | `9000` | refuse a plan with more devices on one relay (r1 maxconn 24000, a relay client holds 2) |
| `--steps` | `2000,5000,10000,15000,20000` | devices online per step |
| `--step-secs`, `--spread-secs` | `240`, `60` | step length, how long a step takes to bring its devices up |
| `--hold-secs` | `900` | hold at the last step |
| `--restart-cmd`, `--restart-at-secs` | none, `600` | run once at this second of the hold, record the reconnect curve |
| `--worst-secs` | `300` | after the hold every device parks |
| `--parked-share`, `--focus-share` | `0.3`, `0.5` | parked (visible window) devices, and how many of them focus their chat |
| `--hold-ms`, `--short-every-secs` | `20000`, `60` | parked hold (15 s on pro), short sync period |
| `--send-every-secs`, `--type-every-secs` | `300`, `120` | 0 turns it off |
| `--direct`, `--relay n=url`, `--pro n=url`, `--origin` | prod names | route URLs |
| `--ock-version`, `--ock-pub` | `1`, the chat-wire pin | OCK the requests are sealed to |
| `--watch name=cmd` | none | abort check, see below |
| `--chat-metrics-cmd`, `--abort-pool-wait-ms` | none, `200` | PG pool wait p99 from chat-api metrics |
| `--music-metrics-cmd`, `--abort-music-rise`, `--baseline-secs` | none, `0.2`, `60` | backend p99 against a baseline taken before the ramp |
| `--abort-error-rate` | `0.01` | harness error rate over 60 s (60 s grace after the restart) |
| `--report` | `chat-load-report.json` | final report |

Routes: `direct` posts to `https://api.scnative.dev/chat/x`, relays to `https://api.<node>.relay.scnative.dev/chat/x`,
pro goes through `https://<node>.pro.scnative.dev/open` and `/take` like the desktop. Plain `http://` routes use h2c.

## Abort checks

A `--watch` command runs every 15 s on the generator. Exit 0 is fine, exit 1 aborts at once with its
last stdout line, anything else three times in a row aborts too. Ready checks in `checks/` (ssh with
`BatchMode`, so the generator needs keys or `ssh -A`):

```sh
--watch "r1-cpu=checks/relay-cpu.sh sc-relay1 75" \
--watch "r2-cpu=checks/relay-cpu.sh sc-relay2 75" \
--watch "r1-queue=checks/haproxy-queue.sh sc-relay1 be_api" \
--watch "r2-queue=checks/haproxy-queue.sh sc-relay2 be_api" \
--watch "chat-rss=checks/container-rss.sh sc-main chat-api 1600" \
--watch "redis-rt=checks/redis-memory.sh sc-main redis-rt 60" \
--chat-metrics-cmd "checks/chat-metrics.sh sc-main" \
--music-metrics-cmd "checks/backend-metrics.sh sc-main"
```

## Report

Every 10 s a status line (online, parked, req/s, errors, sent, received, delivery p50/p99). At the end
`summary` and a JSON report: per route requests, errors by kind (`connect`, `timeout`, `outer_503`,
`chat_busy`, ...), `wake` (delivered to a poll that was parked when the message was sent), `catchup`
(delivered on a later poll, short-sync devices land here), call latency, the reconnect curve after
the restart (parked polls back to 95%, share of devices that talked to the new process at 5..180 s)
and a 5 s timeline. Verdict: wake p99 under 2 s, errors under 1%, back within 60 s after the restart.
chat-api CPU and RSS, relay CPU and music p99 come from the checks and Grafana.

## Local run (podman, 5k devices)

```sh
podman network create lt-net
podman run -d --name lt-pg --network lt-net -e POSTGRES_USER=scd -e POSTGRES_PASSWORD=dev -e POSTGRES_DB=scd \
  public.ecr.aws/docker/library/postgres:17-alpine -c max_connections=200
for f in api/migrations/*.sql api/migrations-ops/*.sql; do podman exec -i lt-pg psql -U scd -d scd -q < "$f"; done
podman exec lt-pg psql -U scd -d scd -c "CREATE TABLE _sqlx_migrations (version bigint PRIMARY KEY, description text NOT NULL,
  installed_on timestamptz NOT NULL DEFAULT now(), success boolean NOT NULL, checksum bytea NOT NULL, execution_time bigint NOT NULL);
  INSERT INTO _sqlx_migrations VALUES (157, 'chat', now(), true, '\x00', 0)"
podman run -d --name lt-rt --network lt-net mirror.gcr.io/library/redis:7-alpine
```

Keys dir: `ock-1` (the `ock_secret` of `utils/chat-wire/vectors/request.json`), random 32 bytes in
`kek-1` and `reports-kek-1`, any token in `chat-admin-token`. Run the release `api` binary in a
`rust:1-bookworm` container on `lt-net` as `lt-chat-api` with `API_ROLE=rt CHAT_LOADTEST=on
CHAT_KEYS_DIR=/keys DATABASE_URL=postgres://scd:dev@lt-pg:5432/scd REDIS_URL=redis://lt-rt:6379
REDIS_RT_URL=redis://lt-rt:6379`. Put `haproxy:3.0-alpine` in front as `lt-edge` like main does
(`bind :8000`, backend `server chat-api lt-chat-api:3000 proto h2`, `http-reuse always`,
`tune.h2.be.max-concurrent-streams 200`): without it every device holds its own h2 connection to
chat-api and its memory measures connection buffers, not chat. The harness runs in another container
on `lt-net`:

```sh
chat-load fake-pro --bind 0.0.0.0:8080 &
chat-load seed --admin http://lt-chat-api:3000 --token-file /keys/chat-admin-token --users 5000 --groups 220 --out layout.json
chat-load run --layout layout.json --ock-pub dde332903df0f9e9fbc91a814131b5bcea28b46f5728e2a7e28700ed4a6c2d4d \
  --direct http://lt-edge:8000 --relay r1=http://lt-edge:8000 --relay r2=http://lt-edge:8000 \
  --pro p1=http://127.0.0.1:8080 --origin http://lt-edge:8000 \
  --steps 1000,2500,5000 --step-secs 60 --spread-secs 30 --hold-secs 240 --worst-secs 90 \
  --restart-at-secs 120 --restart-cmd "curl -s --unix-socket /run/podman/podman.sock -X POST http://d/v4.0.0/libpod/containers/lt-chat-api/restart?t=20"
chat-load purge --admin http://lt-chat-api:3000 --token-file /keys/chat-admin-token
```

Give the harness and haproxy containers `--ulimit nofile=250000:250000`. The podman VM is shared
with other work: a busy VM shows up as Redis commands over 10 ms in `SLOWLOG` and `chat_busy`
answers, so read a local verdict next to `SLOWLOG GET`. Remove `lt-*` containers, the network and
volumes after.

## Night run (plan section 8)

Generator `sc-worker1`, its worker stopped, `ulimit -n 200000`, `ip_local_port_range 10000 65535`.

1. `tools/loadtest/exempt.sh on 144.31.212.127` (Infra).
2. `CHAT_LOADTEST=on` on chat-api, recreate it.
3. Seed on main (the token never leaves the host), copy the layout to the generator:

```sh
ssh sc-main 'ip=$(docker inspect -f "{{range .NetworkSettings.Networks}}{{.IPAddress}} {{end}}" $(docker ps -q --filter label=com.docker.compose.service=chat-api) | cut -d" " -f1);
  curl -fsS -X POST -H "x-chat-admin-token: $(cat /srv/scd-secrets/chat/chat-admin-token)" -H "x-chat-admin-user: loadtest" \
  -H "content-type: application/json" -d "{\"users\":20000,\"devices_per_user\":1,\"groups\":900}" \
  http://$ip:3000/chat-admin/loadtest/seed' > layout.json && chmod 600 layout.json
scp layout.json sc-worker1:
```

4. On the generator:

```sh
./chat-load run --layout layout.json --restart-cmd "ssh sc-main 'cd /root && docker compose restart chat-api'" \
  --watch ... --chat-metrics-cmd ... --music-metrics-cmd ... --report night.json
```

5. Restore: purge (same ssh + curl to `/chat-admin/loadtest/purge` until `"pending":0`),
   `CHAT_LOADTEST=off` and recreate chat-api, `exempt.sh off`, start the worker, delete `layout.json`.

Dry run before the night: the same with `--users 50 --groups 3` and
`--steps 50 --step-secs 30 --hold-secs 120 --worst-secs 0`, flag on, no exemption, then purge.

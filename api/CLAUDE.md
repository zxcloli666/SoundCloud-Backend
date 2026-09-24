# SoundCloud-Backend

Rust (axum) request-serving API for a SoundCloud desktop client. Mirrors SoundCloud content into our own catalog,
enriches it (artists/albums/lyrics), embeds it for a vector-based recommendation "wave", and streams audio. Built for *
*high load**: ~5M tracks, many concurrent users + background jobs.

## Stack & data stores

- **Postgres** (sqlx, queries are compile-time-checked `query_file!` macros — see **Database queries** below) — source
  of truth (`tracks`, `artists`, `albums`, `users`, likes/history/events, `wanted_tracks`,
  `lyrics_cache`). Pool is small (`PG_POOL_MAX`, ~20–50); `max_connections=200` shared across services. **Connections
  are precious — never hold one across network/RPC work.** There are **two** Postgres roles — see **Two databases**.
- **Qdrant** — vector search. Collections: `tracks_mert` (1024d audio), `tracks_clap` (512d audio), `tracks_lyrics` (
  1024d), `tracks_collab` (128d, item2vec). Point id = `sc_track_id`.
- **NATS JetStream** — work queues between backend ↔ worker (enrich, embed, transcribe, index, storage events). See
  `bus/`.
- **Redis** — caches, wave cursors, single-flight locks, rate budgets.
- **MinIO/S3** — transcoded audio (`soundcloud_tracks_<id>.m4a`).
- Sibling services (separate repos/images): **streaming** (SC→S3 download/transcode), **worker** (Python:
  whisper/demucs/embeddings/LLM RPC over NATS), **call** (relay service for SC reads —
  `../SoundCloud-Internal`), **proxy-systems** (`../Proxy-Systems`: intermediate + simple + ipv6 rotating
  proxies for external APIs), **tls-common** (`utils/tls-common`, shared TLS/ACME/PROXY-protocol).

## Track lifecycle (the spine)

`ingest` (like/playlist/discovery → `indexing::ingest_track_from_sc`, UPSERT `tracks`, priority set) → **storage** (
`streaming` downloads from SC → S3; `storage_state`) → **index** (worker embeds audio+lyrics → Qdrant; `index_state`) →
**enrich** (link artists/albums; `enrich_state`) → **lyrics** (jobs: relay-backed external lookup; worker получает только plain→synced align). Each stage has its own
state column + pickup. The bottleneck in prod is **SC download** (rate limits) — mitigated by the `call` relay + rotating
proxies.

The **wave** (`recommendations::smart_wave`) blends 3 arms — track-arm (clap+mert+lyrics NN from seed likes),
artist-arm (affinity graph), collab-arm (item2vec) — and degrades to popularity when a user's taste isn't indexed yet.
So wave quality depends on the user's liked tracks being **indexed** (vectors), and artist pages on tracks being *
*enriched** (linked).

## High-load conventions (FOLLOW THESE)

- **NATS consumers run concurrently, bounded.** `bus/nats.rs::consume(stream, durable, filter, concurrency, handler)`
  spawns handlers up to `concurrency` (permit-before-pull backpressure, ack on completion, ack_wait 120s). Pick
  concurrency per consumer (enrich = `ENRICH_CONSUMER_CONCURRENCY`; quick done-handlers ~16). Never go back to awaiting
  handlers serially.
- **Never hold a pooled DB connection across `.await` on network/RPC/slow work.** (This was the enrich
  `pg_advisory_lock` bug → pool exhaustion at 1 track/min.) Dedup via in-memory `mini_moka` cache + **idempotent UPSERT
  ** (`ON CONFLICT`) + freshness checks, not session locks. Acquire a connection only for the query, release
  immediately.
- **Parallelize bounded fan-out with `FuturesUnordered` or a semaphore-backed stream**, not serial `for x { ...await... }`
  and not unbounded `join_all`. Network work owns no PostgreSQL connection.
- **API не держит внешних каталожных клиентов.** MusicBrainz/Genius/lyrics-агрегаторы живут только в `jobs`; serving-процесс ходит наружу лишь в SoundCloud через `ScReadService`.
- **Mass external reads** run only in jobs through **relay Lua → relay raw → own proxy**. A semantic miss stops the
  transport chain and never poisons relay health; ban/429/5xx/transport/malformed advances to the next tier. Relay
  saturation backpressures instead of spilling a burst onto the smaller proxy pool. Force `Accept-Encoding: identity`.
- **Reading from SoundCloud** — do NOT call `ScClient::api_get_value` directly for a public read.
  Public reads go through the `ScReadService` facade (`sc/read.rs`): a 3-tier chain **apiv2 via relay
  (Lua) → apiv2 via proxy&relay → apiv1 (direct→proxy&relay, token, lazy)**, normalized to apiv1 shape.
  Owner `/me/*` private reads and all writes stay on apiv1 + the user's token; writes go via `sync_queue`.
  Adding an apiv2 endpoint? **curl-test it first** (some are 401/404 for anon). Full rules + how to write a
  new SC Lua method: **[docs/sc-networking.md](docs/sc-networking.md)**.
- **Prioritize user-relevant work.** `TrackPriority` (Like=1 … Discovery=5) → `tracks.{index,storage}_priority`; enrich
  backfill orders by `index_priority` too. Likes/owned must beat the discovery firehose for SC-download/index/enrich.
- **Skip pointless external work.** MusicBrainz only for ISRC/`metadata_artist` (label) tracks — it never matches
  underground SC uploads and its throttle serializes enrich.
- **Tracks > `MAX_TRACK_DURATION_SEC` (7 min)** are terminal `too_long` (storage/index = `too_long`, transcribe
  `disabled`) — not downloaded/indexed (DJ sets/podcasts bloat S3, useless for the wave). Frontend shows an `F` badge.
- **Every `/admin/*` route is gated per-handler by the `AdminAuth` extractor** (`common/admin.rs`: constant-time
  `x-admin-token` vs `config.admin.token`, fail-closed). There is **no** global auth layer in `router.rs` — a new admin
  handler MUST take `_: AdminAuth` as its **first** argument or the route is open to the world (this is how
  `/admin/collab/*` leaked). Body extractors (`Json`, `Option<Json<…>>`) are `FromRequest` and must stay **last**.
- **Комментариев в коде нет** — ни `//`, ни `///`, ни `//!`, ни в SQL, TOML, Dockerfile и CI. Смысл несут имена, типы и структура; объяснения идут в `docs/` и в чат.

## Gotchas (verified in prod)

- **Proxy strips `Content-Encoding` without decompressing** → gzip/br bodies arrive as garbage; logged only at `debug`.
  Always send `Accept-Encoding: identity` for proxied fetches. Fixed in `proxy-common/headers.rs` (forces identity) +
  backend-side external fetching now lives entirely in `jobs`.
- **`call` relay** must reach its control endpoint; the call server expects PROXY-protocol only
  from haproxy. Internal services connect direct (docker alias, bypassing haproxy) → tls-common does optional
  PROXY-detect + trusts only `TLS_PROXY_TRUSTED_HOSTS=haproxy` (auto-resolved). Port `:444` is the desktop's direct
  gRPC (DNAT preserves client IP).

## Module map (`src/modules/`)

`indexing` (ingest + pipeline kick + reaps), `tracks` (repository/UPSERT/projection), `enrich` (только админские ручки и
DTO — резолвер, кравл и wanted-резолвер живут в `jobs`), `admin` (read/CRUD + durable enqueue; тяжёлый maintenance — job `admin.*` в `jobs`), `lyrics` (cache/state read, local preview и durable lookup wake;
LRCLIB/Musixmatch/Genius lookup и align decision живут в `jobs`), `recommendations` (`smart_wave`, arms, blender, cursors,
clusters, bandits, trainer), `collab`/`centroids` (vectors), `cold_refresh` (TTL-based SC re-sync), `auth`/
`oauth_apps` (SC token chains + proxy), `sync_queue` (write-back to SC), `resolve` (`/resolve` handler → `ScReadService`), read-path:
`search discover albums artists playlists users me likes dislikes history auras featured subscriptions`. Infra: `bus/` (
nats), `cache/`, `db/`, `qdrant/`, `redis/`, `sc/` (`ScClient` transport + `ScReadService` public-read facade +
`apiv2`/`mapping`/`lua_methods` — see [docs/sc-networking.md](docs/sc-networking.md)), `common/`, `config.rs`, `main.rs`.

## Two databases: core + ops (FOLLOW THESE)

Prod runs three nodes: **main** и **star** отдают API без кронов (`RESERVE_BACKEND` / `PREMIUM_RESERVE`),
**load** гоняет все фоновые пайплайны и ходит в main по mTLS. Отсюда разделение схемы:

| | core | ops |
|---|---|---|
| env | `DATABASE_URL` (или `DATABASE_HOST/PORT/…`) | `OPS_DATABASE_URL` (или `OPS_DATABASE_*`) |
| обязательна | да | **нет** — не задана ⇒ фичи выключены |
| миграции | `migrations/` (`0000+`) | `migrations-ops/` (`9000+`) |
| что лежит | всё, что нужно отдаче | только то, что отдача не читает |
| где в проде | main (реплика на star) | локальный PG на load |

- **Критерий переноса в ops: таблицу не читает ни один serving-путь.** Пишет её отдача или крон —
  неважно. Сейчас там `rec_impressions` (вектор фич LTR на каждый показанный трек) и
  `rec_hard_negatives` — чистые обучающие выборки, их никто не читает ради ответа юзеру.
- **Доступ только через `db::OpsDb`.** `ops.pool()` даёт `Option<&PgPool>`; `None` — тихо выйти.
  Никаких `unwrap`, никакой заглушечной БД: main/star стартуют вообще без ops.
- **Компиляция видит ОБЕ схемы в одной базе.** `query_file!` проверяется против `DATABASE_URL`,
  поэтому билд-БД получает `migrations/*.sql` **и** `migrations-ops/*.sql` (в этом порядке —
  ops пересоздаёт то, что core у себя дропает). Рантайм разводит их по разным пулам.
- **Диапазоны версий не пересекаются** (`0000+` vs `9000+`), поэтому оба набора могут ужиться в одной
  базе на общем `_sqlx_migrations` — это дефолт локалки и валидный all-in-one деплой. Ради этого у
  обоих мигратов `ignore_missing = true`; проверка контрольных сумм (`VersionMismatch`) при этом жива.
- **mTLS работает для обеих БД** и с обеими формами конфига: `{,OPS_}DATABASE_SSL_{MODE,CA,CERT,KEY}`
  накладываются поверх того, что уже есть в URL. Дали `SSL_CA` без `SSL_MODE` → `verify-full`
  (иначе mTLS молча деградировал бы до sqlx-дефолта `prefer`).

## Database queries (sqlx — FOLLOW THESE)

SQL is **checked against a real Postgres schema at compile time**: a query that selects a dropped/renamed column, or
binds a wrong-typed param, fails `cargo build` — not at 3am in prod. (This is the whole point — it once caught a live bug
where `sync_queue` inserted a `payload` column that migration `0019` had dropped.)

- SQL lives in **`api/queries/<module>/<name>.sql`** (one query per file), **not** inline strings in Rust.
  Ops-запросы живут там же — разводит их не путь, а пул, в который их исполняют.
- Call it with a `query_file*!` macro:
    - `sqlx::query_file_scalar!("queries/<m>/x.sql", arg1, …)` — SELECT of **one column** → `T` / `Option<T>` / `Vec<T>`.
    - `sqlx::query_file_as!(MyRow, "queries/<m>/x.sql", …)` — SELECT into a `#[derive(sqlx::FromRow)]` struct; the `.sql`
      column list must match the struct fields **by name AND order** (expand `SELECT *` to an explicit list).
    - `sqlx::query_file!("queries/<m>/x.sql", …)` — INSERT/UPDATE/DELETE (+`RETURNING`; read `row.<col>`).
- At compile time the macro connects to `DATABASE_URL` and asks Postgres for the query's types — that connection **is**
  the check. **There is NO `.sqlx` offline cache** (deliberately `.gitignore`d — we don't commit that). → **you must
  have a Postgres with migrations applied to build.**

**Build/test locally** — bring up a dev DB once, then `cargo check` validates every macro against it:

```
podman run -d --name scd-dev-pg -e POSTGRES_USER=scd -e POSTGRES_PASSWORD=dev -e POSTGRES_DB=soundcloud_desktop \
  -p 127.0.0.1:55432:5432 docker.io/library/postgres:17-alpine
# ОБА набора, в этом порядке — билд-схема = core ∪ ops.
for f in migrations/*.sql migrations-ops/*.sql; do podman exec -i scd-dev-pg psql -U scd -d soundcloud_desktop < "$f"; done
export DATABASE_URL=postgres://scd:dev@127.0.0.1:55432/soundcloud_desktop
cargo check --all-targets        # macros are validated against this schema
```

CI and `docker build` do the same automatically (spin up an ephemeral Postgres → apply migrations → build online).
Nothing to commit, nothing to keep in sync.

**Keep on runtime `sqlx::query(...)`** — these CANNOT be macros; the reason is always one of:

- **Dynamic SQL** — built with `format!` / conditional `WHERE` / `QueryBuilder` (the string isn't static).
- **`INSERT … VALUES` binding `Option<…>`** — Postgres `DESCRIBE` doesn't report parameter nullability, so the macro
  infers params **NON-NULL** and rejects `Option`. (An UPDATE `SET c = COALESCE($n, c)` IS macro-able — `COALESCE` makes
  `$n` nullable.) The big upserts (`tracks`/`playlists`/`artists`/`albums`) stay runtime for this.
- **`UNNEST($1::int8[], …)` with `Vec<Option<T>>`** (array of nullable elements — macro wants `&[T]`).
- **`pg_stat_*` / system-catalog** queries.

**Nullability hints in the `.sql`:** struct field is `Option<T>` but the column is `NOT NULL` and comes via a
`LEFT JOIN` (so actually nullable) → alias it `AS "col?"`; a non-null expression the macro thinks is nullable
(`COUNT(*)`, `EXISTS(…)`, `(xmax = 0)`) → `AS "col!"`.

**⚠ Formatter trap:** JetBrains "Reformat Code" / Actions-on-Save splits `=>` — `make_interval(days => $n)` becomes
`days = > $n` → SQL syntax error → red CI/Docker. Either disable auto-reformat for `api/queries/**`, or avoid `=>`
(write `$n::int * INTERVAL '1 day'`).

## Migrations (FOLLOW THESE)

`migrations/NNNN_*.sql` (core) и `migrations-ops/9NNN_*.sql` (ops) встраиваются в бинарник
`jobs-migrate`. API никогда не применяет DDL и при старте проверяет обязательную версию core-схемы.
Перед API запускается `migrate core`; на jobs-хосте `migrate ops`, а локальный all-in-one deploy использует
`migrate all`. Двухфазный core-migrator применяет 0059, создаёт или обновляет настроенный OAuth app,
перевязывает legacy connections и только затем завершает оставшиеся миграции. Правила ниже действуют для
обоих наборов; `scripts/check-migrations.sh` проверяет их независимо.

- **A `.sql` edit needs a rebuild+redeploy** to take effect — patching the file and restarting the old binary changes
  nothing. **Never edit an already-applied migration:** the checksum (SHA-384 of the file) lives in `_sqlx_migrations`,
  and any mismatch aborts startup with `VersionMismatch`. Fix forward with a new `NNNN_*.sql`. (Editing a *pending*
  one — not yet in `_sqlx_migrations` — is safe; it applies fresh.) **Enforced** by `scripts/check-migrations.sh`
  (pre-commit `.githooks/pre-commit` + CI `migrations-guard.yml`): rejects edits to a committed migration and
  duplicate/out-of-order numbers; **eugene** lints new migrations for dangerous locks (advisory).
- **One file = one simple-query message = one implicit transaction.** Postgres runs every `;`-separated statement in the
  file as a single transaction, so a migration with ≥2 statements is *always* transactional — `-- no-transaction` does
  **not** change that (it only drops sqlx's own `BEGIN/COMMIT` wrapper).
- **Non-transactional DDL** (`CREATE`/`DROP INDEX CONCURRENTLY`, `REINDEX CONCURRENTLY`, `ALTER TYPE … ADD VALUE`,
  `VACUUM`) cannot run in a transaction. Two ways to ship it:
  - **Default (what every index migration here does):** plain `CREATE INDEX IF NOT EXISTS` in the normal migration, and
    **pre-create the big index `CONCURRENTLY` by hand on prod before deploy** so the migration no-ops via `IF NOT
    EXISTS`. Keep everything idempotent (`IF NOT EXISTS`, `ADD COLUMN IF NOT EXISTS`).
  - **Only when the migration itself must build concurrently:** `-- no-transaction` as the **first line** *and* the file
    holds **exactly one statement** — no `DO` block, no second DDL, nothing else. (A `DO` self-heal block +
    `CREATE INDEX CONCURRENTLY` = two statements = implicit tx = the error that took prod down — `0029`.)
- **A failed `CREATE INDEX CONCURRENTLY` leaves an INVALID index** that `IF NOT EXISTS` then silently skips (planner
  ignores it → seq scans). Drop invalid leftovers by hand; self-heal can't share the file with the concurrent build.
- **Keep deploy migrations cheap.** A heavy in-transaction index build or backfill stalls the rollout — pre-create or
  backfill on prod instead.

## Commands

- Build/check: needs a migrated Postgres (see **Database queries**) — `DATABASE_URL=… cargo check --all-targets` (in
  `api/`). Lint: `cargo clippy --all-targets -- -D warnings` — CI runs latest stable, so keep your toolchain current
  (`rustup update`) or you'll miss newer lints it rejects. Migrations: see **Migrations** above.
- Query plans: `scripts/check-query-plans.sh` (CI `query-plans.yml`) flags `Seq Scan` on big tables (advisory). It only
  means something on a seeded base: `scripts/seed-plan-fixtures.sql` fills 23 big tables with 50000 rows each (dates
  spread over 400 days, realistic `user_events`/likes/credits distributions); `scripts/seed-plan-fixtures-scale.sh`
  does the same at prod row counts with `PSQL=… OVERRIDES='{"user_events":…}'`, dropping and rebuilding the
  non-constraint indexes of the scaled tables around the load. Prod slow queries / unused indexes: `GET /admin/slow-queries`,
  `GET /admin/index-usage` (header `x-admin-token`).
- Key env: `PG_POOL_MAX`, `MAX_TRACK_DURATION_SEC`, `ENRICH_MB_RATE_LIMIT_MS`, `SC_PROXY_URL`, `CALL_*`,
  `TLS_PROXY_TRUSTED_HOSTS`. Lyrics/crawl concurrency and Genius/Musixmatch configuration belong to jobs.
- Prod: compose on dedic `ssh dedic-ru:/root/docker-compose.yml`; DB/qdrant/minio creds in
  `../Infra/main-host/docker-compose.yml`. Query prod DB from PC via
  `podman run ... postgres:17-alpine psql -h <dedic> ...`.

## Session & SC token — читать до правок в auth/me

**`SessionCtx` НЕ материализует SC-токен.** Он даёт `session_id` + `sc_user_id`, и
резолвится через `get_session` без обновления. `/me` читает локальный профиль;
маршрут `/me/cold` удалён. Обновление профиля и устаревших сущностей каталога
принадлежит durable job `catalog.refresh`. В API нет отцепленных задач для этих обновлений.

**Приложение обязано работать с протухшим SC-токеном.** `/me` возвращает зеркало
`user_profiles` либо минимальный профиль из локальных данных. Только разрешённые
SC-пути запрашивают токен через `ctx.with_access_token(...)`.

**Проверка подключения отделена от профиля.** `GET /auth/soundcloud` читает
состояние из PostgreSQL без SC-запроса. Явное обновление — `POST /auth/soundcloud/refresh`.
Общая аренда в `soundcloud_connections` защищает обновления из API и jobs.

**Одиночный `invalid_grant` не требует входа.** Миграция 0089 и общий SQL записи
отказа требуют минимум три отказа за период не менее 15 минут, последнего успеха
не менее 30 минут назад и уже истёкшего access token. Между попытками — 10 минут.
Успех, повторная авторизация и временная ошибка сбрасывают серию отказов.
Таймауты, 429, 5xx и недоступная конфигурация OAuth сохраняют локальную сессию.

Контракт фронтенда: [docs/frontend-api-rewrite.md](../docs/frontend-api-rewrite.md).

## Сборка локально

sqlx проверяет запросы ONLINE, офлайн-кеша `.sqlx` в репо нет. Нужна БД со схемой
из **обеих** папок: `migrations/*` **и** `migrations-ops/*` (без второй падает на
`relation "rec_impressions" does not exist` — миграция 0054 выносит их в ops-БД, а
`events/service.rs` держит на них compile-time запросы). Так же делает CI.

```bash
podman run -d --name scd-pg --network host \
  -e POSTGRES_USER=scd -e POSTGRES_PASSWORD=x -e POSTGRES_DB=scd \
  docker.io/library/postgres:17-alpine
for f in migrations/*.sql migrations-ops/*.sql; do \
  podman exec -i scd-pg psql -U scd -d scd -v ON_ERROR_STOP=1 -q < "$f"; done
DATABASE_URL=postgres://scd:x@127.0.0.1:5432/scd cargo check
```

API проверяет обязательную версию core-схемы и не применяет миграции при старте.
Перед запуском API миграции выполняет `jobs-migrate`. Production rollout описан
в [backend-rewrite-roadmap.md](../docs/backend-rewrite-roadmap.md); до завершения переписи деплой запрещён.

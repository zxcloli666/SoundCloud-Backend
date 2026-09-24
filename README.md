# SoundCloud-Backend

Серверная часть SoundCloud: API, медиа-сервисы и фоновые пайплайны.

## Сервисы

| Папка       | Язык        | Образ (GHCR `ghcr.io/zxcloli666/soundcloud-backend/…`) |
|-------------|-------------|--------------------------------------------------------|
| `api`       | Rust (axum) | `api`                                                  |
| `jobs`      | Rust (axum) | `jobs`, `jobs-migrate`                                 |
| `streaming` | Rust        | `streaming`                                            |
| `storage`   | Rust        | `storage`                                              |
| `worker`    | Python      | `worker:{cpu,gpu,gpu-cuda12}`                          |

## Локальный запуск

```bash
cp .env.example .env   # заполни SOUNDCLOUD_CLIENT_ID/SECRET и пр.
podman compose -f docker-compose-dev.yml up -d
```

`jobs-migrate all` последовательно подготавливает core- и ops-БД. API сам миграции не выполняет и не
запустится на несовместимой схеме. Для раздельного production rollout используются `jobs-migrate core`
на serving-узлах и `jobs-migrate ops` на jobs-узле.

`jobs` создаёт и валидирует обязательные коллекции Qdrant до перехода в ready. API коллекции не меняет
и запускается после ready-сигнала jobs, проверяя их схему ещё раз перед приёмом трафика.

Порты: API `3000`, jobs health `3001`, streaming `8080`, storage `3002`, postgres `5432`, redis `6379`,
qdrant `6333/6334`, nats `4222/8222`.

## CI

- `ci.yml` — clippy/build бэкенда (sqlx online-check на поднятой Postgres), сборка образов
  storage/streaming/worker, prod-сборка с реальными крейтами.
- `auto-build.yml` — публикация образов в GHCR (тег `vX.Y.Z` + `latest`).
- `migrations-guard.yml` — append-only гард миграций. `query-plans.yml` — EXPLAIN-гейт.

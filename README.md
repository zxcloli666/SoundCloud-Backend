# SoundCloud-Backend

Серверная часть SoundCloud: API, медиа-сервисы и фоновые пайплайны.

## Сервисы

| Папка       | Язык        | Образ (GHCR `ghcr.io/zxcloli666/soundcloud-backend/…`) |
|-------------|-------------|--------------------------------------------------------|
| `api`       | Rust (axum) | `api`                                                  |
| `streaming` | Rust        | `streaming`                                            |
| `storage`   | Rust        | `storage`                                              |
| `worker`    | Python      | `worker:{cpu,gpu,gpu-cuda12}`                          |

## Локальный запуск

```bash
cp .env.example .env   # заполни SOUNDCLOUD_CLIENT_ID/SECRET и пр.
podman compose -f docker-compose-dev.yml up -d
```

Порты: backend `3000`, streaming `8080`, storage `3002`, postgres `5432`, redis `6379`,
qdrant `6333/6334`, nats `4222/8222`.

## CI

- `ci.yml` — clippy/build бэкенда (sqlx online-check на поднятой Postgres), сборка образов
  storage/streaming/worker, prod-сборка с реальными крейтами.
- `auto-build.yml` — публикация образов в GHCR (тег `vX.Y.Z` + `latest`).
- `migrations-guard.yml` — append-only гард миграций. `query-plans.yml` — EXPLAIN-гейт.

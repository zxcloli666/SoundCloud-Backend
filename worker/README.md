# worker

Слой моделей над NATS JetStream. Задача приходит из рабочего стрима, результат уходит в `done.*`,
RPC `ai.rpc.*` получает ответ на `X-Reply-To`. Состояние задач держит JetStream, между рестартами
живёт только кэш весов в `/models`.

## Границы

- HTTP-сервера нет. Статус — core-subject `worker.status.<worker_id>` из контракта, здоровье
  контейнера проверяет `HEALTHCHECK` командой `python -m worker health`.
- Исходящий HTTP — только GET аудио по ссылке из задачи и вызовы LLM-провайдеров.
  `fetch-models` ходит в сеть отдельной командой; в образе `HF_HUB_OFFLINE=1`.
- Нет PostgreSQL, Qdrant, S3. Вектор уезжает в `done.*`, в хранилища его пишет backend.
- Стримы и durable-консьюмеры создаёт jobs. Воркер подписывается только `pull_subscribe_bind`:
  `pull_subscribe` и `add_consumer` в `bus/` запрещены тестом `tests/unit/test_layers.py`.

## Контракт

`contract/worker-contract.json` — единственный источник лейнов, схем запросов и ответов, причин,
correlation-ключей и окон (`deadline_s`, `ack_wait_s`, `max_deliver`, задержки `nak`). В TOML этих
чисел нет. В образе файл лежит в `/app/contract/worker-contract.json`.

| Лейн | Стрим | Subject | Ответ | Публичная нода |
|---|---|---|---|---|
| `audio` | `INDEX_AUDIO` | `index.audio.new` | `done.index_audio` | да |
| `lyrics` | `EMBED_LYRICS` | `embed.lyrics.new` | `done.embed_lyrics` | да |
| `transcribe` | `TRANSCRIBE` | `transcribe.audio.new` | `done.transcribe` | да |
| `encode` | `ENCODE` | `encode.text.new` | `done.encode` | нет |
| `collab` | `TRAIN_COLLAB` | `train.collab.new` | `done.train_collab` | нет |
| `taste` | `TRAIN_TASTE` | `train.taste.new` | `done.train_taste` | нет |
| `ai` | `AI_RPC` | `ai.rpc.>` | ответ на `X-Reply-To` | нет |

## Конфигурация

Три слоя, каждый следующий перекрывает предыдущий:

1. `config/worker.toml` — все ключи с дефолтами;
2. `config/profiles/<WORKER_PROFILE>.toml`;
3. переменные `WORKER__<SECTION>__<KEY>`, например `WORKER__LANES__CAPACITY='{ audio = 8 }'`.

Неизвестный ключ — ошибка старта. Секреты задаются только ссылкой `"env:NAME"`:

| Переменная | Для чего |
|---|---|
| `WORKER_PROFILE` | имя профиля |
| `WORKER_NODE_NAME` | `worker.id`, попадает в `producer.worker_id` и subject статуса |
| `NATS_URL`, `NATS_USER`, `NATS_PASSWORD` | подключение к NATS |
| `ANTHROPIC_API_KEY` | основной LLM-провайдер |
| `LLM_FALLBACK_URL`, `LLM_FALLBACK_MODEL`, `LLM_FALLBACK_KEY` | запасной OpenAI-совместимый провайдер |

Провайдер LLM включён тогда и только тогда, когда его ключ непустой.

### Профили

| Профиль | Режим | Лейны (ёмкость) | Реплики sep/asr/align/mms | Для кого |
|---|---|---|---|---|
| `gpu-12` | `lane` | audio 4, lyrics 8, transcribe 1 | 1/1/1/1 | волонтёр, 12 ГБ без рабочего стола |
| `gpu-12-audio` | `lane` | audio 8, lyrics 8 | — | волонтёр под массовый индекс, 12 ГБ с рабочим столом |
| `gpu-24` | `slot` | audio 16, lyrics 32, transcribe 4, encode 32, collab 1, taste 1, ai 64 | 2/2/2/1 | доверенный хост, 24 ГБ |
| `gpu-24-lane` | `lane` | как `gpu-24` | 2/2/2/1 | резерв `gpu-24`, если исполнитель в режиме `slot` не помещается в RAM |
| `gpu-48` | `slot` | audio 32, lyrics 64, transcribe 8, encode 64, collab 1, taste 1, ai 64 | 4/4/4/4 | 48 ГБ; локальный LLM запасным провайдером |
| `cpu` | `lane` | lyrics 1, encode 1, collab 1, ai 16 | — | хост без GPU, oneDNN выключен |

`gpu-12*` работают с `trust = "public"`: непубличный по контракту лейн в `lanes.enabled` —
ошибка старта (`settings.check_public_lanes`), остальное ограничивают права ноды на брокере.
У `gpu-12*` `idle_unload_s = 600` и батчи из столбца «12 ГБ».
Резервы включены во всех профилях: `align → mms`, `sep → mix`, регионный повтор, gap-fill,
`rescue_strategy = "global_ctc"`, `oom_unload`, запасной LLM-провайдер. Это держат тесты
`tests/unit/test_config_shipped.py` и `tests/contract/test_profiles.py`.

## Образ

Один `docker/Dockerfile`, сборка torch выбирается `FLAVOR`:

| Тег | `FLAVOR` | torch | Драйвер |
|---|---|---|---|
| `:gpu`, `:gpu-cuda12`, `:X.Y.Z-gpu` | `cu126` | 2.13, Pascal–Ada | ≥ R560 |
| `:gpu-cuda13`, `:X.Y.Z-gpu-cuda13` | `cu130` | 2.14, Turing+ и Blackwell | ≥ R580 |
| `:cpu`, `:X.Y.Z-cpu` | `cpu` | 2.13 | — |

`:gpu-cuda13` экспериментальный: на доверенный хост и в пул нод не выдаётся, пока models-тесты на
нём не прогнаны на реальной карте. CUDA приезжает колёсами `nvidia-*`, драйвер — с хоста.

```bash
docker build -f docker/Dockerfile --build-arg FLAVOR=cu126 -t worker:gpu .
podman build --format docker -f docker/Dockerfile --build-arg FLAVOR=cu126 -t worker:gpu .
```

`podman build` без `--format docker` молча выбрасывает `HEALTHCHECK`.

Контейнер: пользователь uid 10001, `read_only: true`, tmpfs `/tmp` и `/run/worker:mode=1777`, тома
`models:/models` и `work:/work`, `shm_size: 4g`, `stop_grace_period: 90s`, секреты через `env_file`,
один контейнер на GPU. Podman монтирует tmpfs без `mode` как `root 0755` (кроме `/tmp`), и воркер
не может записать `/run/worker/health.json`: процесс выходит с кодом 1 сразу после старта.

### Веса

Веса в образ не входят. Перед первым запуском:

```bash
docker compose run --rm -e HF_HUB_OFFLINE=0 worker fetch-models --profile gpu-24
```

Команда качает ревизии из конфига только для слотов включённых лейнов профиля (с резервами),
fastText `lid.176.bin` и чекпойнт сепаратора сверяет по sha256. Код выхода: 0 — всё скачано,
1 — часть не скачалась (каждая ошибка в строке `model_fetch_failed`), 78 — ошибка конфига.

## Разработка

```bash
uv sync
.venv/bin/ruff format --check . && .venv/bin/ruff check . && .venv/bin/mypy
.venv/bin/python -m pytest
NATS_TEST_URL=nats://127.0.0.1:4222 .venv/bin/python -m pytest tests/integration
```

`uv sync` без флагов ставит группы `dev` и `cu126`; сборка под CPU — `uv sync --no-default-groups
--group cpu --group dev`.

| Уровень | Папка | Без чего пропускается |
|---|---|---|
| unit | `tests/unit` | — |
| runtime | `tests/runtime` | — |
| bus | `tests/bus` | — |
| contract | `tests/contract` | — |
| integration | `tests/integration` | `NATS_TEST_URL` |
| models | `tests/models` | `/dev/nvidia0` |
| eval | `tests/eval` | `EVAL_DATA_DIR` |

Правила для кода — в [AGENTS.md](./AGENTS.md).

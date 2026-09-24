# worker — правила

## Что это

Слой моделей над NATS JetStream. Задача приходит из рабочего стрима, результат уходит в `done.*`,
RPC — ответом на `X-Reply-To`. Устройство, профили и образ — в [README.md](./README.md).

## Границы

- Никакой бизнес-логики: ранжирование, бюджеты LLM, запись в БД и векторные хранилища — у backend.
- HTTP-сервера нет. Исходящий HTTP — только GET аудио по ссылке из задачи и LLM-провайдеры.
  Веса качает отдельная команда `fetch-models`; в образе `HF_HUB_OFFLINE=1`.
- Нет PostgreSQL, Qdrant, S3.
- Стримы и консьюмеры создаёт jobs. Воркер подписывается только `pull_subscribe_bind`;
  `pull_subscribe` и `add_consumer` запрещены.
- Контракт — `contract/worker-contract.json`. Окна лейнов, схемы и причины берутся оттуда, в TOML
  их нет. Изменение провода начинается в `../backend-contracts`, потом здесь.

## Код

- Комментариев и docstring'ов нет нигде: ни в `.py`, ни в TOML, ни в Dockerfile, ни в YAML.
  Объясняют имена, размер функций и структура модуля.
- Читается сверху вниз: публичное первым, помощники ниже.
- Без абстракций «на будущее», без TODO, без мёртвого кода, без шимов совместимости.
- Каждая ошибка обрабатывается явно: счётчик и строка лога, ни одного `except: pass`.
- Резервные пути включены в поставляемом конфиге и во всех профилях. «Работает без резерва» —
  не довод выключить резерв.

## Слои

Проверяет `tests/unit/test_layers.py`:

- `bus` импортирует только `contract`, `settings`, `domain.outcome`, `domain.deadline`,
  `observability.counters`;
- `domain` не импортирует `nats`, `bus`, `runtime`, `llm`; движки и хранилище получает через
  `domain/ports.py`;
- `runtime` не импортирует `domain`, `bus`, `nats`;
- torch и прочий нативный ML-код (fastText, chromaprint, gensim, silero, transformers) — только в
  `models/*` и в модулях исполнителя `runtime/{engine_main,allocator,devices}.py`. Процесс с
  NATS-циклом их не импортирует; ffmpeg — дочерний процесс.

## Проверка

```bash
.venv/bin/ruff format --check . && .venv/bin/ruff check . && .venv/bin/mypy
.venv/bin/python -m pytest
```

Тесты с GPU (`tests/models`, eval) делят карту с рабочим столом: после прогона память свободна,
фоновых процессов нет.

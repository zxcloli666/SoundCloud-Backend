# Storage

## Назначение

Сервис хранения транскодированных треков. Принимает сырое аудио, транскодит (ffmpeg), кладёт в backend (local FS, S3 или Google Drive), отдаёт по ссылке.

## Роль в архитектуре

Storage — единственная точка, где аудио физически живёт и раздаётся. Но **разные потребители получают разные URL**:

- **Desktop-клиенты** — всегда идут **через storage** (`http://storage/...`). Бэкенд-URL (S3/Drive) наружу никогда не светится. Причины: контроль доступа, ротация бэкенда без изменений в клиенте, единый origin для CDN/прокси на фронте.
- **Worker (AI)** — получает **URL на storage** (`{storage}/{file}.m4a`), storage сам стримит байты. `/redirect/{file}.m4a` (307 на presigned S3 URL) остаётся как резервный путь.

Таргет редиректа зависит от бэкенда:
- **S3** — свежий presigned URL (TTL 15 мин), scope = bucket+key+method.
- **Gdrive** — `https://drive.google.com/uc?export=download&id=...`, файлу проставляется ACL `anyone:reader` лениво при первом запросе. Без TTL, scope ограничен одним файлом; revoke = удалить файл или снять `anyone`-permission.
- **Local** — stream-through, как `serve`.

## Правила

- **Клиенты не должны знать про бэкенд.** Никогда не отдавать S3/Drive URL в desktop-ответах. Только свой origin storage.
- **Воркер получает `{storage}/{file}.m4a`.** `/redirect/...` только для совместимости и резерва.
- **Бэкенд хранилища абстрагирован** (`backend/local.rs`, `backend/s3.rs`, `backend/gdrive.rs`). Переключение — через `STORAGE_BACKEND` env (`local` | `s3` | `gdrive`). `/redirect/` работает на всех.
//! Stable, named IDs для `pg_advisory_lock`. Все ID — статика; ни один не
//! пересекается с другим. Используется для координации singleton-cron'ов
//! между инстансами бекенда.
//!
//! Для per-entity tx-level lock'ов (replace-операции типа `playlist_tracks`
//! reorder) используем `pg_advisory_xact_lock(hashtext($1))` с осмысленным
//! строковым ключом — см. callers.

/// Lock на запуск sqlx-миграций core-БД: только один процесс держит его на старте.
pub const MIGRATIONS: i64 = 0x5343445F4D49;

/// То же для ops-БД. Отдельный ID нужен на случай, когда ops указывает на ту же
/// базу, что и core: два набора миграций тогда не борются за один lock.
pub const MIGRATIONS_OPS: i64 = 0x5343445F4D4F;

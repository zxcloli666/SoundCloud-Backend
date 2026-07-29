-- `last_read_at` пишется (touch_last_read, раз в 6 часов на сущность), но не
-- читается: ни один запрос не фильтрует и не сортирует по нему. Оно ставилось
-- под cold-eviction (`COLD_EVICT_AFTER_SEC`), который так и не поехал —
-- `ColdCfg::evict_after_sec` до сих пор `#[allow(dead_code)]`.
--
-- Индекс на такой колонке — чистый write-amplification: не-HOT update пишет во
-- ВСЕ индексы таблицы, так что он дорожает каждую перезапись строки, а не
-- только touch. На 34-ГБ `tracks` это заметно. Колонку и запись оставляем —
-- seam под eviction дешёвый; убираем только индексы.
--
-- Pre-dropped CONCURRENTLY на проде; DROP IF EXISTS здесь no-op'ит.
DROP INDEX IF EXISTS tracks_last_read_at_idx;
DROP INDEX IF EXISTS users_last_read_at_idx;
DROP INDEX IF EXISTS playlists_last_read_at_idx;

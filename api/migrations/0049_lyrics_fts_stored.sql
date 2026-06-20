-- Материализуем FTS-вектор лирики в колонку. Было: `ts_rank(to_tsvector(coalesce||
-- regexp_replace(synced_lrc)), q)` в ORDER BY пере-вычислял тяжёлое выражение на КАЖДУЮ
-- из ~20k совпавших строк (частые слова) → ~20с → упор в statement_timeout 2.5с → пустой
-- UI. Теперь @@ и ts_rank читают готовый tsvector.
-- Колонка nullable (instant, без переписи таблицы — в отличие от GENERATED STORED, см. 0029).
-- Поддерживается триггером; существующие строки — backfill отдельно (scripts/backfill-lyrics-fts.sql).
-- GIN-индекс pre-create CONCURRENTLY на проде (no-op по IF NOT EXISTS). Старый
-- lyrics_cache_fts_gin (expression) после деплоя не нужен — дропнуть отдельным шагом.
ALTER TABLE lyrics_cache ADD COLUMN IF NOT EXISTS fts tsvector;

CREATE OR REPLACE FUNCTION lyrics_cache_fts_refresh() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    NEW.fts := to_tsvector(
        'simple',
        coalesce(NEW.plain_text, '') || ' '
        || regexp_replace(coalesce(NEW.synced_lrc, ''), '\[[0-9:.]+\]', ' ', 'g')
    );
    RETURN NEW;
END $$;

DROP TRIGGER IF EXISTS lyrics_cache_fts_trg ON lyrics_cache;
CREATE TRIGGER lyrics_cache_fts_trg
    BEFORE INSERT OR UPDATE OF plain_text, synced_lrc ON lyrics_cache
    FOR EACH ROW EXECUTE FUNCTION lyrics_cache_fts_refresh();

CREATE INDEX IF NOT EXISTS lyrics_cache_fts_col_gin ON lyrics_cache USING GIN (fts);

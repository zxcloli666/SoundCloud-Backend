-- Одноразовый backfill lyrics_cache.fts на проде (новые/изменённые строки заполняет триггер).
-- ~95k строк, один UPDATE (row-locks). Прогнать ПОСЛЕ миграции 0049 и до/сразу-после деплоя
-- нового бинаря (старые строки с fts IS NULL не найдутся, пока не залиты). При желании дробить по id.
UPDATE lyrics_cache
SET fts = to_tsvector(
    'simple',
    coalesce(plain_text, '') || ' '
    || regexp_replace(coalesce(synced_lrc, ''), '\[[0-9:.]+\]', ' ', 'g')
)
WHERE fts IS NULL;

-- 0053: индекс по tracks(genius_song_id) — обратный поиск трека по песне Genius
-- (дедуп связки, аналитика). Partial: genius_song_id заполнен у меньшинства треков.
-- genius_url НЕ индексируем — по нему не фильтруем (пишется/читается по PK трека).
--
-- OPS: пред-создать CONCURRENTLY на проде ДО деплоя → миграция no-op:
--   CREATE INDEX CONCURRENTLY IF NOT EXISTS tracks_genius_song_id_idx
--       ON tracks (genius_song_id) WHERE genius_song_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS tracks_genius_song_id_idx
    ON tracks (genius_song_id)
    WHERE genius_song_id IS NOT NULL;

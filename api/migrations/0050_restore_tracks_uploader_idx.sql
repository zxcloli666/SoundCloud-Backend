-- 0050: восстановить btree по tracks(uploader_sc_user_id).
-- 0044 дропнул tracks_uploader_idx как «префикс tracks_uploader_artist_idx» — ошибка:
-- тот индекс PARTIAL (WHERE primary_artist_id IS NOT NULL), не покрывает голый
-- WHERE uploader_sc_user_id = $1. Без него все такие фильтры (admin artist_detail,
-- enrich reupload/repoint, search) = Seq Scan по 10.7М строк.
--
-- OPS: пред-создать CONCURRENTLY на проде ДО деплоя → миграция no-op:
--   CREATE INDEX CONCURRENTLY IF NOT EXISTS tracks_uploader_idx
--       ON tracks (uploader_sc_user_id) WHERE uploader_sc_user_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS tracks_uploader_idx
    ON tracks (uploader_sc_user_id)
    WHERE uploader_sc_user_id IS NOT NULL;

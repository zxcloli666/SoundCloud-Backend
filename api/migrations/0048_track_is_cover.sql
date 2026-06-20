-- is_cover: предвычисленный флаг «трек — кавер/реап, не оригинал артиста».
-- Схлопывает дорогую read-time логику вкладок артиста (upload_kind/cover_of_artist_id/
-- non-native uploader, OR+EXISTS → seq-scan по tracks) в один булев-фильтр.
-- Пишется при enrich (finalize_track) и при ингесте (тег (cover) в тайтле).
-- Бэкфилл существующих строк — отдельным шагом на проде (scripts/backfill-is-cover.sql),
-- НЕ в boot-миграции: апдейт 1.5M строк под migration-локом застопорит флот.
-- Индекс: pre-create CONCURRENTLY на проде до деплоя → миграция no-op'ит по IF NOT EXISTS.
ALTER TABLE tracks ADD COLUMN IF NOT EXISTS is_cover boolean NOT NULL DEFAULT false;
CREATE INDEX IF NOT EXISTS tracks_is_cover_idx ON tracks (is_cover) WHERE is_cover;

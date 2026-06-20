-- Одноразовый бэкфилл tracks.is_cover на проде. Запускать ВРУЧНУЮ (не в boot-миграции).
-- Обновляет только подходящие строки (меньшинство) → row-locks, не table-lock.
-- Совпадает с логикой записи is_cover в enrich (finalize_track) и ингесте (тег в тайтле).
-- Дробить по диапазонам id, если разово окажется тяжело.
UPDATE tracks
SET is_cover = true
WHERE NOT is_cover
  AND (COALESCE(upload_kind, '') IN ('cover', 'reupload')
       OR cover_of_artist_id IS NOT NULL
       OR title ~* '[\(\[]\s*cover(\s+version)?\s*[\)\]]');

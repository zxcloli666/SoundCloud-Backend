CREATE INDEX IF NOT EXISTS tracks_quality_upgrade_idx
    ON tracks (storage_quality, hq_upgrade_last_at NULLS FIRST)
    WHERE hq_upgrade_pending = true;

DROP INDEX IF EXISTS tracks_hq_upgrade_idx;

UPDATE tracks
SET hq_upgrade_pending = true
WHERE storage_state = 'ok'
  AND storage_quality = 'hq'
  AND hq_upgrade_pending = false;

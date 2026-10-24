UPDATE tracks
SET storage_state   = CASE
                          WHEN storage_state IN ('ok', 'missing') THEN 'pending'
                          ELSE storage_state
    END,
    storage_quality = CASE
                          WHEN storage_state IN ('ok', 'missing') THEN NULL
                          ELSE storage_quality
    END,
    s3_verified_at  = NULL,
    s3_missing_at   = now()
WHERE sc_track_id = ANY ($1)
  AND storage_state <> 'too_long'

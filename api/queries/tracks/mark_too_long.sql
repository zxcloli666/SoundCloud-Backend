UPDATE tracks
SET storage_state    = 'too_long',
    index_state      = 'too_long',
    indexed_at       = NULL,
    transcribe_state = 'disabled',
    hq_upgrade_pending = false,
    updated_at       = now()
WHERE sc_track_id = $1
  AND (
      storage_state <> 'too_long'
      OR index_state <> 'too_long'
      OR transcribe_state IS DISTINCT FROM 'disabled'
      OR hq_upgrade_pending
  )

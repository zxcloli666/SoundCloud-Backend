INSERT INTO storage_event_repairs (sc_track_id, uploaded_at)
VALUES ($1, $2)
ON CONFLICT (sc_track_id) DO UPDATE
SET uploaded_at = GREATEST(storage_event_repairs.uploaded_at, EXCLUDED.uploaded_at),
    repaired_at = now()

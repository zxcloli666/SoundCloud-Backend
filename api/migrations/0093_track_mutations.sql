ALTER TABLE tracks
    ADD COLUMN IF NOT EXISTS sc_metadata jsonb NOT NULL DEFAULT '{}',
    ADD COLUMN IF NOT EXISTS deleted_at timestamptz;

UPDATE sync_queue
SET action_type = 'track_update',
    payload = jsonb_build_object('track', jsonb_build_object('sharing', payload->>'sharing')),
    generation = generation + 1,
    remote_attempted_generation = NULL,
    remote_completed_generation = NULL,
    remote_result = NULL,
    next_run_at = clock_timestamp()
WHERE action_type = 'track_sharing';

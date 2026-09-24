ALTER TABLE playlists
    ADD COLUMN IF NOT EXISTS sc_metadata jsonb NOT NULL DEFAULT '{}',
    ADD COLUMN IF NOT EXISTS deleted_at timestamptz;

WITH deleted AS MATERIALIZED (
    SELECT playlist.urn, nextval('catalog_metadata_clock') AS sequence
    FROM playlists AS playlist
    WHERE EXISTS (
        SELECT 1 FROM sync_queue
        WHERE action_type = 'playlist_delete' AND target_urn = playlist.urn
    )
)
UPDATE playlists AS playlist
SET deleted_at = COALESCE(playlist.deleted_at, clock_timestamp()),
    sharing = 'private',
    sc_desired = '{}',
    sc_observation = deleted.sequence,
    sc_mutation_observation = deleted.sequence,
    sc_write_confirmed = false
FROM deleted
WHERE playlist.urn = deleted.urn;

UPDATE playlist_membership_state AS state
SET next_reconcile_at = NULL,
    reconcile_generation = reconcile_generation + 1
FROM playlists AS playlist
WHERE playlist.urn = state.playlist_urn AND playlist.deleted_at IS NOT NULL;

UPDATE sync_queue
SET action_type = 'playlist_update',
    payload = jsonb_build_object('playlist', jsonb_build_object('sharing', payload->>'sharing')),
    generation = generation + 1,
    remote_attempted_generation = NULL,
    remote_completed_generation = NULL,
    remote_result = NULL,
    next_run_at = clock_timestamp()
WHERE action_type = 'playlist_sharing';

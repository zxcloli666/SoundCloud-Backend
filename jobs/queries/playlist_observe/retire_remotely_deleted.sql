WITH observation AS MATERIALIZED (
    SELECT public.scd_clock() AS sequence
), gone AS (
    UPDATE playlists AS playlist
    SET sharing = 'private', sc_desired = '{}',
        deleted_at = COALESCE(playlist.deleted_at, clock_timestamp()),
        sc_observation = observation.sequence,
        sc_mutation_observation = observation.sequence,
        sc_write_confirmed = true,
        updated_at = clock_timestamp()
    FROM observation
    WHERE playlist.urn = $1 AND playlist.owner_sc_user_id = $2
    RETURNING playlist.urn
), cancelled AS (
    SELECT count(*) AS purged
    FROM sync_queue AS queued
    JOIN gone ON queued.target_urn = gone.urn
    WHERE queued.user_id = $2
      AND public.scd_purge_intent('sync_queue', jsonb_build_object('id', queued.id), queued.intent_v)
), retired AS (
    UPDATE playlist_membership_state AS state
    SET next_reconcile_at = NULL, reconcile_generation = state.reconcile_generation + 1,
        updated_at = clock_timestamp()
    FROM gone
    WHERE state.playlist_urn = gone.urn
), disowned AS (
    DELETE FROM user_owned_playlists AS owned
    USING gone
    WHERE owned.playlist_urn = gone.urn AND owned.user_id = ANY ($3)
)
SELECT gone.urn AS "urn!"
FROM gone
CROSS JOIN cancelled

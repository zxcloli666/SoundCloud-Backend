INSERT INTO playlist_membership_state (
    playlist_urn,
    owner_sc_user_id,
    projection_track_count,
    sync_status,
    next_reconcile_at
)
VALUES (
    $1,
    coalesce((SELECT owner_sc_user_id FROM playlists WHERE urn = $1), ''),
    (
        SELECT count(*)::integer
        FROM playlist_track_projection
        WHERE playlist_urn = $1
    ),
    'unhydrated',
    clock_timestamp()
)
ON CONFLICT (playlist_urn) DO UPDATE
SET owner_sc_user_id = EXCLUDED.owner_sc_user_id,
    projection_track_count = EXCLUDED.projection_track_count,
    sync_status = CASE
        WHEN playlist_membership_state.baseline_generation = 0 THEN 'unhydrated'
        ELSE playlist_membership_state.sync_status
    END,
    next_reconcile_at = coalesce(
        playlist_membership_state.next_reconcile_at,
        clock_timestamp()
    ),
    updated_at = clock_timestamp()

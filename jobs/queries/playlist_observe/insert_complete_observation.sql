INSERT INTO playlist_remote_observations (
    playlist_urn,
    snapshot_id,
    authority,
    outcome,
    pagination_complete,
    all_items_identified,
    declared_track_count,
    observed_track_count,
    sc_last_modified,
    observed_at
)
VALUES ($1, $2, 'owner', 'complete', true, true, $3, $3, $4, $5)
RETURNING id

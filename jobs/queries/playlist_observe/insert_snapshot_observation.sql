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
    observed_at,
    error_kind
)
VALUES ($1, $2, $3, $4, true, true, $5, $6, $7, $8, $9)
RETURNING id

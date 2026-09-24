INSERT INTO playlist_remote_observations (
    playlist_urn,
    authority,
    outcome,
    pagination_complete,
    all_items_identified,
    observed_at,
    retry_at,
    error_kind
)
VALUES ($1, 'owner', $2, false, false, $3, $4, $5)
RETURNING id

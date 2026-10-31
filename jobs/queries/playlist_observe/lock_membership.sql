UPDATE playlist_membership_state
SET updated_at = updated_at
WHERE playlist_urn = $1
RETURNING playlist_urn

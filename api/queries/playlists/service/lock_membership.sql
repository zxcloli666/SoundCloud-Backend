SELECT playlist_urn FROM playlist_membership_state
WHERE playlist_urn = $1 FOR UPDATE

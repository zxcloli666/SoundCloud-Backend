SELECT projection_revision
FROM playlist_membership_state
WHERE playlist_urn = $1
FOR UPDATE

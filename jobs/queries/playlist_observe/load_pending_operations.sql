SELECT operation_id,
       sequence,
       kind,
       track_id,
       left_anchor_track_id,
       right_anchor_track_id,
       boundary,
       array_remove(ordered_track_ids, NULL) AS ordered_track_ids
FROM playlist_membership_operations
WHERE playlist_urn = $1
  AND resolved_at IS NULL
  AND sequence <= $2
ORDER BY sequence

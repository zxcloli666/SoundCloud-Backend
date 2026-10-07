SELECT t.sc_track_id
FROM tracks t
WHERE t.sc_track_id = ANY ($1)
  AND t.uploader_sc_user_id IN (SELECT unnest(b.sc_user_ids)
                                FROM user_blocked_artists b
                                WHERE b.sc_user_id = ANY ($2))

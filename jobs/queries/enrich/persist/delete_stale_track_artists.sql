DELETE
FROM track_artists AS credit
WHERE credit.track_id = $1
  AND (NOT EXISTS (SELECT 1
                   FROM unnest($2::uuid[], $3::text[]) AS kept(artist_id, role)
                   WHERE kept.artist_id = credit.artist_id
                     AND kept.role = credit.role)
    OR (public.scd_derived() AND EXISTS (SELECT 1
                                         FROM track_artist_blocks AS block
                                         WHERE block.track_id = credit.track_id
                                           AND block.artist_id = credit.artist_id)))

SELECT track.sc_track_id AS "sc_track_id!",
       CASE
           WHEN star.tier = 1 THEN 1
           WHEN track.index_priority = $2 THEN 2
           WHEN star.tier = 3 THEN 3
           ELSE 4
       END::smallint AS "rank!"
FROM tracks AS track
LEFT JOIN star_track_tiers AS star
  ON star.sc_track_id = track.sc_track_id
WHERE track.sc_track_id = ANY ($1::text[])

INSERT INTO wanted_track_albums (wanted_track_id, album_id, position)
SELECT placement.wanted_track_id, placement.album_id, placement.position
FROM UNNEST($1::uuid[], $2::uuid[], $3::smallint[])
    AS placement(wanted_track_id, album_id, position)
ON CONFLICT DO NOTHING

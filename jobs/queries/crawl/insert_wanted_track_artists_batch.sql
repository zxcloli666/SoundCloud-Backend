INSERT INTO wanted_track_artists (wanted_track_id, artist_id, role, position)
SELECT credit.wanted_track_id, credit.artist_id, credit.role, credit.position
FROM UNNEST($1::uuid[], $2::uuid[], $3::text[], $4::smallint[])
    AS credit(wanted_track_id, artist_id, role, position)
ON CONFLICT DO NOTHING

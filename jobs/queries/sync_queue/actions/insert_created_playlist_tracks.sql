INSERT INTO playlist_track_projection (playlist_urn, position, sc_track_id)
SELECT $1, position - 1, sc_track_id
FROM unnest($2::text[]) WITH ORDINALITY AS tracks(sc_track_id, position)

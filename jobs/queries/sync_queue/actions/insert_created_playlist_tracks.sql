INSERT INTO playlist_track_projection (playlist_urn, position, sc_track_id)
SELECT $1, row_number() OVER (ORDER BY earliest.position) - 1, earliest.sc_track_id
FROM (
    SELECT sc_track_id, min(position) AS position
    FROM unnest($2::text[]) WITH ORDINALITY AS tracks(sc_track_id, position)
    GROUP BY sc_track_id
) AS earliest

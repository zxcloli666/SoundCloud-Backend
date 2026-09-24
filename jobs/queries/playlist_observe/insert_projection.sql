INSERT INTO playlist_track_projection (
    playlist_urn,
    position,
    sc_track_id
)
SELECT $1,
       (entry.ordinality - 1)::integer,
       entry.sc_track_id
FROM unnest($2::text[]) WITH ORDINALITY AS entry(sc_track_id, ordinality)

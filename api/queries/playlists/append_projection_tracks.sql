INSERT INTO playlist_track_projection (
    playlist_urn,
    position,
    sc_track_id
)
SELECT $1,
       (
           SELECT coalesce(max(existing.position), -1)
           FROM playlist_track_projection AS existing
           WHERE existing.playlist_urn = $1
       )::integer + entry.ordinality::integer,
       entry.sc_track_id
FROM unnest($2::text[]) WITH ORDINALITY AS entry(sc_track_id, ordinality)

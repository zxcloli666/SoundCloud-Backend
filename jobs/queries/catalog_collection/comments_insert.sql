WITH page AS (
    SELECT DISTINCT ON (item->>'id') item, ordinal
    FROM jsonb_array_elements($4::jsonb) WITH ORDINALITY AS delivered(item, ordinal)
    ORDER BY item->>'id', ordinal
)
INSERT INTO track_comments (
    id, sc_track_id, sc_comment_id, user_urn, body, track_position_ms, sc_created_at, created_at, synced_at
)
SELECT gen_random_uuid(),
       $1,
       item->>'id',
       item->'user'->>'urn',
       item->>'body',
       (item->>'timestamp')::bigint,
       (item->>'created_at')::timestamptz,
       $2::timestamptz - ($3::bigint + ordinal)::double precision * interval '1 microsecond',
       $2::timestamptz
FROM page
ON CONFLICT (sc_comment_id) DO UPDATE
SET body = EXCLUDED.body,
    track_position_ms = EXCLUDED.track_position_ms,
    sc_created_at = EXCLUDED.sc_created_at,
    synced_at = EXCLUDED.synced_at

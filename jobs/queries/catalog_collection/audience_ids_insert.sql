INSERT INTO catalog_audience_ids (subject_id, relation, user_id, created_at)
SELECT $1::bigint, $2::smallint, substring(item.key FROM '^soundcloud:users:([0-9]{1,18})$')::bigint,
       $3::timestamptz - ($4::bigint + item.ordinal)::double precision * interval '1 microsecond'
FROM unnest($5::text[]) WITH ORDINALITY AS item(key, ordinal)
WHERE item.key ~ '^soundcloud:users:[0-9]{1,18}$'
ON CONFLICT (subject_id, relation, user_id) DO NOTHING

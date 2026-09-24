INSERT INTO catalog_audience (subject_urn, relation, user_urn, created_at)
SELECT $1, $2, item.key,
       $3::timestamptz - ($4::bigint + item.ordinal)::double precision * interval '1 microsecond'
FROM unnest($5::text[]) WITH ORDINALITY AS item(key, ordinal)
ON CONFLICT (subject_urn, relation, user_urn) DO NOTHING

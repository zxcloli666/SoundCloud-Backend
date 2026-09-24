INSERT INTO user_taste_vectors (sc_user_id, version, vec, updated_at)
SELECT users.sc_user_id,
       $2,
       ($3::real[])[(users.ordinal - 1) * $4::int + 1 : users.ordinal * $4::int],
       now()
FROM unnest($1::text[]) WITH ORDINALITY AS users (sc_user_id, ordinal)
ON CONFLICT (sc_user_id, version) DO UPDATE
SET vec = EXCLUDED.vec,
    updated_at = EXCLUDED.updated_at

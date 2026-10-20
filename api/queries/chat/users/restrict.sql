INSERT INTO chat.users AS u (user_id, restricted_until, restriction_scope)
VALUES ($1, $2, $3)
ON CONFLICT (user_id) DO UPDATE
    SET restriction_scope = CASE
            WHEN u.restricted_until > now()
                THEN ARRAY(SELECT DISTINCT scope FROM unnest(u.restriction_scope || EXCLUDED.restriction_scope) AS scope ORDER BY scope)
            ELSE EXCLUDED.restriction_scope
        END,
        restricted_until = CASE
            WHEN u.restricted_until > now() THEN GREATEST(u.restricted_until, EXCLUDED.restricted_until)
            ELSE EXCLUDED.restricted_until
        END,
        updated_at = now()
RETURNING restricted_until AS "restricted_until!", restriction_scope

WITH root AS (
    INSERT INTO chat.media (id, store, object_key, owner_user, origin_conversation, kind, upload_id, bytes, status, delete_after)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'reserved', now() + $12::int * interval '1 second')
    ON CONFLICT (owner_user, upload_id) WHERE status = 'reserved' AND kind <> 'thumb' DO NOTHING
    RETURNING id, store, owner_user, origin_conversation
),
thumb AS (
    INSERT INTO chat.media (id, store, object_key, owner_user, origin_conversation, kind, parent_id, bytes, status, delete_after)
    SELECT $9::uuid, root.store, $10::text, root.owner_user, root.origin_conversation, 'thumb', root.id, $11::bigint,
           'reserved', now() + $12::int * interval '1 second'
    FROM root
    WHERE $9::uuid IS NOT NULL
    RETURNING id
)
SELECT (SELECT count(*) FROM root) AS "roots!",
       (SELECT count(*) FROM thumb) AS "thumbs!"

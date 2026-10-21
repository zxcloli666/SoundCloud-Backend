SELECT admin AS "admin!", action AS "action!", detail AS "detail!", created_at AS "created_at!"
FROM (
    SELECT id, admin, action, detail, created_at
    FROM chat.admin_audit
    WHERE target = $1
       OR (detail ->> 'report') = $2
    ORDER BY created_at DESC, id DESC
    LIMIT $3
) newest
ORDER BY created_at, id

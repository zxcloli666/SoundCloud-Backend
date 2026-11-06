SELECT id,
       admin,
       action,
       target,
       detail,
       created_at
FROM chat.admin_audit
WHERE action LIKE 'public.%'
  AND ($1::text IS NULL OR admin = $1)
  AND ($2::uuid IS NULL OR id < $2)
ORDER BY id DESC
LIMIT $3

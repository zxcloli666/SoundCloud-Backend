SELECT admin, action, detail, created_at
FROM chat.admin_audit
WHERE target = $1
ORDER BY created_at
LIMIT $2

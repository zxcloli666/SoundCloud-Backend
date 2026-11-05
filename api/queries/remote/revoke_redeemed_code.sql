DELETE FROM remote_grants
WHERE code_hash = $1
  AND access_hash IS NOT NULL
RETURNING id

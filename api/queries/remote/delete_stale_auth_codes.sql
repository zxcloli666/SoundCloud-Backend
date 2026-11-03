DELETE FROM remote_grants
WHERE access_hash IS NULL
  AND external_id IS NULL
  AND code_expires_at < now()

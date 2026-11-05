DELETE FROM remote_grants
WHERE provider = 'salut' AND external_id = $1

UPDATE video_accounts
SET secret = $2
WHERE name = $1 AND secret NOT LIKE 'v1:%'

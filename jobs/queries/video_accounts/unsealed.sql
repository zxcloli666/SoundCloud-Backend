SELECT name, secret
FROM video_accounts
WHERE secret NOT LIKE 'v1:%'

SELECT count(*) AS "pending!"
FROM chat.conversations
WHERE created_by LIKE 'lt:%'
   OR dm_key LIKE '%lt:%'

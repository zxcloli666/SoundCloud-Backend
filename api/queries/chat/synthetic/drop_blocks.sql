DELETE FROM chat.blocks
WHERE user_id LIKE 'lt:%'
   OR blocked_user LIKE 'lt:%'

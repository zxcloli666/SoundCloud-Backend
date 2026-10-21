DELETE FROM chat.reports
WHERE reporter LIKE 'lt:%'
   OR target_user LIKE 'lt:%'

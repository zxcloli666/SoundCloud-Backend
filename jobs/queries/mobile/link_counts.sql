SELECT CASE plan
           WHEN 'high_tier' THEN 'go_plus'
           WHEN 'mid_tier' THEN 'go'
           ELSE 'free'
       END AS "plan!",
       CASE
           WHEN status = 'relink_required' THEN 'relink_required'
           WHEN expires_at > now() THEN 'working'
           ELSE 'stale'
       END AS "state!",
       count(*)::bigint AS "links!"
FROM mobile_credentials
GROUP BY 1, 2

SELECT (SELECT count(*) FROM chat.users) AS "users!",
       (SELECT count(*) FROM chat.users WHERE first_seen_at > now() - interval '1 day') AS "new_day!",
       (SELECT count(*) FROM chat.users WHERE restricted_until > now()) AS "restricted!",
       (SELECT count(DISTINCT user_id) FROM chat.devices WHERE last_seen_at > now() - interval '1 day') AS "active_day!",
       (SELECT count(DISTINCT user_id) FROM chat.devices WHERE last_seen_at > now() - interval '7 days') AS "active_week!",
       (SELECT count(*) FROM chat.devices WHERE status = 'active') AS "devices!",
       (SELECT count(*) FROM chat.reports WHERE state = 'open') AS "reports_open!"

SELECT COALESCE(account.username, public.auth_session_username($2)) AS username,
       account.avatar_url
FROM (SELECT $1::text AS sc_user_id) AS me
LEFT JOIN users AS account ON account.sc_user_id = me.sc_user_id

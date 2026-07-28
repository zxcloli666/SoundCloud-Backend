SELECT sc_user_id,
       urn,
       username,
       permalink,
       avatar_url,
       country,
       city,
       verified,
       followers_count
FROM users
WHERE sc_user_id = ANY ($1)

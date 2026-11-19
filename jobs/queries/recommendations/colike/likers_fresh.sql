CREATE TEMP TABLE colike_likers AS
SELECT artist_id, count(*) AS likers
FROM colike_users
GROUP BY artist_id

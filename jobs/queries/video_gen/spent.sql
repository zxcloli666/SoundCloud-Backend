SELECT COALESCE(sum(usd), 0)::float8 AS "usd!"
FROM video_gen_spend
WHERE period = $1

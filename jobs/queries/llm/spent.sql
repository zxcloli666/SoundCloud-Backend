SELECT COALESCE(sum(usd), 0)::float8 AS "usd!"
FROM llm_spend
WHERE ladder = $1
  AND period = $2

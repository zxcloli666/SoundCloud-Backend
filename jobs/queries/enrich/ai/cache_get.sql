SELECT reply
FROM ai_resolver_cache
WHERE cache_key = $1
  AND expires_at > now()

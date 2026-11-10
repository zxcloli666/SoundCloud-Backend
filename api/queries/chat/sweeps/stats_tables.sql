SELECT relname::text AS name,
       n_live_tup AS rows,
       pg_total_relation_size(relid) AS bytes
FROM pg_stat_user_tables
WHERE schemaname = 'chat'

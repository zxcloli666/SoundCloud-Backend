SELECT pg_relation_size('lyrics_lookup_state'::regclass) / current_setting('block_size')::bigint AS "pages!"

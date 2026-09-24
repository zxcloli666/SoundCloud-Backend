SELECT COUNT(*)::int8 AS "total!",
       COUNT(*) FILTER (WHERE index_state = 'indexed')::int8 AS "indexed!"
FROM tracks

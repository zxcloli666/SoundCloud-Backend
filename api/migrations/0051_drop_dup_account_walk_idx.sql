-- 0051: снести мёртвые дубли account-walk индексов на artists.
-- claim.sql фильтрует has_sc_account → используется только artists_account_walk_walkable_idx
-- (0035, partial WHERE merged_into IS NULL AND has_sc_account; 2.3M сканов на проде).
-- artists_account_walk_pickup_idx (0016) и artists_account_walk_claim_idx (0032) —
-- идентичные (last_account_walk_at NULLS FIRST WHERE merged_into IS NULL), 0 сканов,
-- по 23MB, чистый write-amp на 1.2M-таблице. 0035 планировал замену, но не дропнул.
-- OPS: на проде снести CONCURRENTLY ДО деплоя → миграция no-op:
--   DROP INDEX CONCURRENTLY IF EXISTS artists_account_walk_pickup_idx;
--   DROP INDEX CONCURRENTLY IF EXISTS artists_account_walk_claim_idx;
DROP INDEX IF EXISTS artists_account_walk_pickup_idx;
DROP INDEX IF EXISTS artists_account_walk_claim_idx;

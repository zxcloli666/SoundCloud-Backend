-- rec_impressions / rec_hard_negatives уехали в ops-БД (`migrations-ops/9000`).
-- Из core их выносим: отдаче они не нужны, а `features` jsonb на каждый
-- показанный трек — самый жирный писатель на main.
--
-- ⚠ ПОРЯДОК ДЕПЛОЯ (main<->star — active-active): накатывать, когда ОБЕ ноды
-- уже на образе, который в эти таблицы не пишет. Данные, если нужны, слить в
-- ops ДО деплоя — см. Infra/docs/postgres-cluster.md.
-- В публикации `scd_pub` их и так нет, так что DROP репликацию не ломает.
DROP TABLE IF EXISTS "rec_impressions";
DROP TABLE IF EXISTS "rec_hard_negatives";

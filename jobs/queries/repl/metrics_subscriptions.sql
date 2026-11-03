SELECT subscription.subname::text AS "sub!",
       (subscription.subenabled AND worker.pid IS NOT NULL) AS "up!",
       coalesce(greatest(pg_wal_lsn_diff(worker.received_lsn, worker.latest_end_lsn), 0), 0)::float8 AS "backlog_bytes!",
       CASE
           WHEN coalesce(pg_wal_lsn_diff(worker.received_lsn, worker.latest_end_lsn), 0) > 0
               THEN coalesce(extract(epoch FROM now() - worker.latest_end_time), 0)
           ELSE 0
       END::float8 AS "age_seconds!"
FROM pg_catalog.pg_subscription AS subscription
LEFT JOIN pg_catalog.pg_stat_subscription AS worker
  ON worker.subid = subscription.oid
 AND worker.relid IS NULL
 AND worker.leader_pid IS NULL
WHERE subscription.subdbid = (
          SELECT database.oid FROM pg_catalog.pg_database AS database WHERE database.datname = current_database()
      )
  AND public.scd_mode() = 'split'
  AND public.scd_role() = 'core'

SELECT count(*) FILTER (WHERE subscription.subenabled) AS "enabled!",
       count(*) FILTER (
           WHERE subscription.subenabled
             AND (
                 worker.pid IS NULL
                 OR worker.last_msg_receipt_time IS NULL
                 OR worker.last_msg_receipt_time < now() - interval '60 seconds'
             )
       ) AS "stalled!"
FROM pg_catalog.pg_subscription AS subscription
LEFT JOIN pg_catalog.pg_stat_subscription AS worker
  ON worker.subid = subscription.oid
 AND worker.relid IS NULL
 AND worker.leader_pid IS NULL
WHERE subscription.subdbid = (
          SELECT database.oid FROM pg_catalog.pg_database AS database WHERE database.datname = current_database()
      )

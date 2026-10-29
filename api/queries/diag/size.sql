SELECT (pg_total_relation_size('diag_bundles')
      + pg_total_relation_size('diag_events')
      + pg_total_relation_size('diag_issue_days')
      + pg_total_relation_size('diag_issue_origins'))::bigint AS "bytes!"

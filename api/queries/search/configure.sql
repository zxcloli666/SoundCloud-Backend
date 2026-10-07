SELECT set_config('statement_timeout', '1500', true) AS "statement_timeout",
       set_config('work_mem', '32MB', true) AS "work_mem",
       set_config('plan_cache_mode', 'force_custom_plan', true) AS "plan_cache_mode",
       set_config('enable_seqscan', 'off', true) AS "enable_seqscan"

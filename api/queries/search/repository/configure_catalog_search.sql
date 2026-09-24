SELECT set_config('statement_timeout', $1, true) AS statement_timeout,
       set_config('plan_cache_mode', 'force_custom_plan', true) AS plan_cache_mode

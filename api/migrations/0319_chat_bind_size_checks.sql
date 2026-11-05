CREATE OR REPLACE FUNCTION public.migration_cancellable(p_backend_types text[], p_queries text[])
RETURNS boolean
LANGUAGE sql
IMMUTABLE
SET search_path = pg_catalog, public, pg_temp
AS $f$
SELECT coalesce(cardinality(p_backend_types), 0) > 0
       AND 'autovacuum worker' = ALL (p_backend_types)
       AND NOT EXISTS (
               SELECT 1
               FROM unnest(p_queries) AS blocked(query)
               WHERE blocked.query IS NULL
                  OR blocked.query NOT LIKE 'autovacuum: %'
                  OR blocked.query LIKE '%(to prevent wraparound)%'
           )
$f$;

REVOKE EXECUTE ON FUNCTION public.migration_cancellable(text[], text[]) FROM PUBLIC;

CREATE OR REPLACE FUNCTION public.migration_lock_tables(p_tables regclass[], p_mode text, p_tries int)
RETURNS void
LANGUAGE plpgsql
SET search_path = pg_catalog, public, pg_temp
AS $f$
DECLARE
    v_timeout text := current_setting('lock_timeout');
    v_table regclass;
    v_failed regclass;
    v_tries integer := 0;
    v_pids integer[];
    v_types text[];
    v_queries text[];
    v_blockers text;
    v_conflicts text[] := CASE p_mode
        WHEN 'ACCESS EXCLUSIVE' THEN ARRAY['AccessShareLock', 'RowShareLock', 'RowExclusiveLock', 'ShareUpdateExclusiveLock',
                                           'ShareLock', 'ShareRowExclusiveLock', 'ExclusiveLock', 'AccessExclusiveLock']
        WHEN 'SHARE UPDATE EXCLUSIVE' THEN ARRAY['ShareUpdateExclusiveLock', 'ShareLock', 'ShareRowExclusiveLock',
                                                 'ExclusiveLock', 'AccessExclusiveLock']
    END;
BEGIN
    IF v_conflicts IS NULL THEN
        RAISE EXCEPTION 'migration_lock_tables takes ACCESS EXCLUSIVE or SHARE UPDATE EXCLUSIVE, not %', p_mode;
    END IF;
    LOOP
        BEGIN
            FOREACH v_table IN ARRAY p_tables LOOP
                v_failed := v_table;
                BEGIN
                    EXECUTE format('LOCK TABLE %s IN %s MODE NOWAIT', v_table, p_mode);
                EXCEPTION WHEN lock_not_available THEN
                    PERFORM pg_catalog.set_config('lock_timeout', '30ms', true);
                    EXECUTE format('LOCK TABLE %s IN %s MODE', v_table, p_mode);
                END;
            END LOOP;
            PERFORM pg_catalog.set_config('lock_timeout', v_timeout, true);
            RETURN;
        EXCEPTION WHEN lock_not_available THEN
            v_tries := v_tries + 1;
            SELECT array_agg(holder.pid),
                   array_agg(coalesce(activity.backend_type, 'prepared transaction')),
                   array_agg(activity.query),
                   string_agg(format('pid %s %s %s %s, xact age %s, query age %s',
                                     holder.pid, coalesce(activity.backend_type, 'prepared transaction'), holder.mode,
                                     CASE WHEN holder.granted THEN 'held' ELSE 'waiting' END,
                                     clock_timestamp() - activity.xact_start,
                                     clock_timestamp() - activity.query_start), '; ')
            INTO v_pids, v_types, v_queries, v_blockers
            FROM pg_catalog.pg_locks AS holder
            LEFT JOIN pg_catalog.pg_stat_activity AS activity ON activity.pid = holder.pid
            WHERE holder.locktype = 'relation'
              AND holder.relation = v_failed
              AND holder.pid IS DISTINCT FROM pg_catalog.pg_backend_pid()
              AND holder.mode = ANY (v_conflicts);
            RAISE NOTICE 'lock try % of % on % in % mode failed: %',
                v_tries, p_tries, v_failed, p_mode, coalesce(v_blockers, 'no blocker visible');
            IF v_tries >= p_tries THEN
                RAISE EXCEPTION 'could not lock % in % mode after % tries, last blocker: %',
                    v_failed, p_mode, v_tries, coalesce(v_blockers, 'none visible')
                    USING ERRCODE = 'lock_not_available';
            END IF;
            IF public.migration_cancellable(v_types, v_queries) THEN
                PERFORM pg_catalog.pg_cancel_backend(activity.pid)
                FROM pg_catalog.pg_stat_activity AS activity
                WHERE activity.pid = ANY (v_pids)
                  AND public.migration_cancellable(ARRAY[activity.backend_type], ARRAY[activity.query]);
            ELSE
                PERFORM pg_catalog.pg_sleep(1);
            END IF;
        END;
    END LOOP;
END
$f$;

REVOKE EXECUTE ON FUNCTION public.migration_lock_tables(regclass[], text, int) FROM PUBLIC;

DO $do$
DECLARE
    v_table regclass;
    v_name text;
    v_oversized boolean;
BEGIN
    PERFORM pg_catalog.set_config('lock_timeout', '10min', true);
    FOREACH v_table IN ARRAY ARRAY['public.sessions', 'public.login_requests', 'public.link_requests']::regclass[] LOOP
        EXECUTE format('SELECT EXISTS (SELECT 1 FROM %s WHERE octet_length(chat_bind) > 512)', v_table)
            INTO v_oversized;
        IF v_oversized THEN
            RAISE EXCEPTION '% holds a chat bind over 512 bytes', v_table
                USING HINT = 'set those binds to NULL on every node, then rerun the core migrations';
        END IF;
    END LOOP;

    PERFORM public.migration_lock_tables(
        ARRAY['public.login_requests', 'public.link_requests', 'public.sessions']::regclass[],
        'ACCESS EXCLUSIVE',
        120
    );

    FOREACH v_table IN ARRAY ARRAY['public.login_requests', 'public.link_requests', 'public.sessions']::regclass[] LOOP
        v_name := (SELECT relation.relname FROM pg_catalog.pg_class AS relation WHERE relation.oid = v_table)
                  || '_chat_bind_size';
        CONTINUE WHEN EXISTS (
            SELECT 1
            FROM pg_catalog.pg_constraint AS constraint_row
            WHERE constraint_row.conrelid = v_table
              AND constraint_row.conname = v_name
        );
        EXECUTE format('ALTER TABLE %s ADD CONSTRAINT %I CHECK (octet_length(chat_bind) <= 512) NOT VALID',
                       v_table, v_name);
    END LOOP;
END
$do$;

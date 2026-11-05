DO $do$
DECLARE
    v_table regclass;
    v_name text;
    v_oversized boolean;
    v_tries integer := 0;
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

    LOOP
        BEGIN
            FOREACH v_table IN ARRAY ARRAY['public.login_requests', 'public.link_requests', 'public.sessions']::regclass[] LOOP
                BEGIN
                    EXECUTE format('LOCK TABLE %s IN ACCESS EXCLUSIVE MODE NOWAIT', v_table);
                EXCEPTION WHEN lock_not_available THEN
                    RAISE EXCEPTION 'experiment: 0319 NOWAIT contended on %, blockers %', v_table,
                        (SELECT string_agg(activity.pid || ' ' || coalesce(activity.backend_type, '') || ' ' || coalesce(left(activity.query, 60), ''), '; ')
                         FROM pg_catalog.pg_locks AS held JOIN pg_catalog.pg_stat_activity AS activity ON activity.pid = held.pid
                         WHERE held.relation = v_table AND held.granted AND held.pid <> pg_backend_pid());
                END;
            END LOOP;
            EXIT;
        EXCEPTION WHEN lock_not_available THEN
            v_tries := v_tries + 1;
            IF v_tries = 120 THEN
                RAISE;
            END IF;
            PERFORM pg_catalog.pg_sleep(1);
        END;
    END LOOP;

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

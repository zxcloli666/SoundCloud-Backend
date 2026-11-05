DO $do$
DECLARE
    v_table regclass;
    v_tries integer := 0;
BEGIN
    PERFORM pg_catalog.set_config('lock_timeout', '200ms', true);
    LOOP
        BEGIN
            LOCK TABLE public.login_requests IN SHARE UPDATE EXCLUSIVE MODE;
            LOCK TABLE public.link_requests IN SHARE UPDATE EXCLUSIVE MODE;
            LOCK TABLE public.sessions IN SHARE UPDATE EXCLUSIVE MODE;
            EXIT;
        EXCEPTION WHEN lock_not_available THEN
            v_tries := v_tries + 1;
            IF v_tries = 1 THEN
                RAISE;
            END IF;
            PERFORM pg_catalog.pg_sleep(1);
        END;
    END LOOP;

    FOREACH v_table IN ARRAY ARRAY['public.login_requests', 'public.link_requests', 'public.sessions']::regclass[] LOOP
        EXECUTE format('ALTER TABLE %s VALIDATE CONSTRAINT %I',
                       v_table,
                       (SELECT relation.relname FROM pg_catalog.pg_class AS relation WHERE relation.oid = v_table)
                       || '_chat_bind_size');
    END LOOP;
END
$do$;

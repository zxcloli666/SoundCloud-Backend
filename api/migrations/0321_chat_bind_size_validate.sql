DO $do$
DECLARE
    v_table regclass;
BEGIN
    PERFORM public.migration_lock_tables(
        ARRAY['public.login_requests', 'public.link_requests', 'public.sessions']::regclass[],
        'SHARE UPDATE EXCLUSIVE',
        120
    );
    FOREACH v_table IN ARRAY ARRAY['public.login_requests', 'public.link_requests', 'public.sessions']::regclass[] LOOP
        EXECUTE format('ALTER TABLE %s VALIDATE CONSTRAINT %I',
                       v_table,
                       (SELECT relation.relname FROM pg_catalog.pg_class AS relation WHERE relation.oid = v_table)
                       || '_chat_bind_size');
    END LOOP;
END
$do$;

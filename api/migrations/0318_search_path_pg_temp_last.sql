DO $do$
DECLARE
    v_routine regprocedure;
    v_path text;
BEGIN
    FOR v_routine, v_path IN
        SELECT routine.oid::regprocedure, substr(config, length('search_path=') + 1)
        FROM pg_catalog.pg_proc AS routine,
             unnest(routine.proconfig) AS config
        WHERE routine.pronamespace NOT IN ('pg_catalog'::regnamespace, 'information_schema'::regnamespace)
          AND config LIKE 'search_path=%'
          AND config NOT LIKE '%pg\_temp%'
          AND NOT EXISTS (
                  SELECT 1
                  FROM pg_catalog.pg_depend AS dependency
                  WHERE dependency.classid = 'pg_catalog.pg_proc'::regclass
                    AND dependency.objid = routine.oid
                    AND dependency.deptype = 'e'
              )
    LOOP
        EXECUTE format('ALTER ROUTINE %s SET search_path = %s, pg_temp', v_routine, v_path);
    END LOOP;
END
$do$;

CREATE OR REPLACE FUNCTION public.auth_pin_scd()
RETURNS void
LANGUAGE plpgsql
SET search_path = pg_catalog, public, pg_temp
AS $f$
DECLARE
    v_name text;
    v_value text;
    v_database oid := (SELECT database.oid FROM pg_catalog.pg_database AS database WHERE database.datname = current_database());
    v_role oid := (SELECT role.oid FROM pg_catalog.pg_roles AS role WHERE role.rolname = session_user);
    v_elsewhere text;
BEGIN
    FOREACH v_name IN ARRAY ARRAY['scd.mode', 'scd.node', 'scd.role', 'scd.guard', 'scd.merging', 'scd.cmd', 'scd.writer'] LOOP
        v_value := '';
        IF v_name IN ('scd.mode', 'scd.node', 'scd.role', 'scd.guard') THEN
            SELECT substr(config, length(v_name) + 2) INTO v_value
            FROM pg_catalog.pg_db_role_setting AS setting,
                 unnest(setting.setconfig) AS config
            WHERE setting.setrole = 0
              AND setting.setdatabase = v_database
              AND config LIKE v_name || '=%';
            SELECT substr(config, length(v_name) + 2) INTO v_elsewhere
            FROM pg_catalog.pg_db_role_setting AS setting,
                 unnest(setting.setconfig) AS config
            WHERE setting.setrole = v_role
              AND setting.setdatabase IN (0, v_database)
              AND config LIKE v_name || '=%'
              AND substr(config, length(v_name) + 2) <> '';
            IF v_elsewhere IS NOT NULL THEN
                RAISE EXCEPTION '% is set on the role, not with ALTER DATABASE', v_name
                    USING ERRCODE = '55000', HINT = 'reset it on the role and set it with ALTER DATABASE (public.scd_configure for scd.mode, scd.node and scd.role)';
            END IF;
        END IF;
        PERFORM pg_catalog.set_config(v_name, coalesce(v_value, ''), true);
    END LOOP;
END
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_pin_scd() FROM PUBLIC;

CREATE OR REPLACE FUNCTION public.auth_start_window(p_kind text, p_limit int)
RETURNS void
LANGUAGE plpgsql
SET search_path = pg_catalog, public, pg_temp
AS $f$
DECLARE
    v_oldest timestamptz;
BEGIN
    IF current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'auth starts run in read committed, not %', current_setting('transaction_isolation')
            USING ERRCODE = '25000';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('auth-start-' || p_kind, 0));
    DELETE FROM auth_caps.starts AS start
    WHERE start.kind = p_kind
      AND start.at <= clock_timestamp() - interval '1 minute';
    IF (SELECT count(*) FROM auth_caps.starts AS start WHERE start.kind = p_kind) >= p_limit THEN
        SELECT min(start.at) INTO v_oldest FROM auth_caps.starts AS start WHERE start.kind = p_kind;
        RAISE EXCEPTION '% % starts in the last minute', p_limit, p_kind
            USING ERRCODE = 'SC429',
                  DETAIL = greatest(1, ceil(extract(EPOCH FROM v_oldest + interval '1 minute' - clock_timestamp()) * 1000))::bigint::text;
    END IF;
    INSERT INTO auth_caps.starts (kind, at) VALUES (p_kind, clock_timestamp());
END
$f$;

REVOKE EXECUTE ON FUNCTION public.auth_start_window(text, int) FROM PUBLIC;

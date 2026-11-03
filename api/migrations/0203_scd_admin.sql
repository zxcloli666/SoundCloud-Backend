SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_publication_columns(tbl text, cols text[])
RETURNS text[]
LANGUAGE sql
STABLE
AS $f$
SELECT coalesce(array_agg(a.attname::text ORDER BY a.attname::text COLLATE "C"), '{}')
FROM pg_catalog.pg_attribute a
WHERE a.attrelid = to_regclass('public.' || quote_ident(tbl))
  AND a.attnum > 0 AND NOT a.attisdropped AND a.attgenerated = ''
  AND a.attname::text = ANY (cols)
  AND NOT EXISTS (SELECT 1 FROM public.scd_unpublished_columns u WHERE u.tbl = $1 AND u.col = a.attname::text)
$f$;

CREATE OR REPLACE FUNCTION public.scd_sync_publications()
RETURNS SETOF text
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    side text := CASE public.scd_role() WHEN 'core' THEN 'core' ELSE 'api' END;
    p record;
    r record;
    want text[];
    have text[];
BEGIN
    FOR p IN SELECT DISTINCT l.pubname, l.publish FROM public.scd_publication_lists() l WHERE l.node = side ORDER BY 1 LOOP
        IF NOT EXISTS (SELECT 1 FROM pg_publication x WHERE x.pubname = p.pubname) THEN
            EXECUTE format('CREATE PUBLICATION %I WITH (publish = %L)', p.pubname, p.publish);
            RETURN NEXT format('create publication %s', p.pubname);
        ELSIF NOT EXISTS (
            SELECT 1 FROM pg_publication x
            WHERE x.pubname = p.pubname AND x.pubinsert = (p.publish LIKE '%insert%') AND x.pubupdate = (p.publish LIKE '%update%')
              AND x.pubdelete = (p.publish LIKE '%delete%') AND NOT x.pubtruncate
        ) THEN
            EXECUTE format('ALTER PUBLICATION %I SET (publish = %L)', p.pubname, p.publish);
            RETURN NEXT format('set publish %s = %s', p.pubname, p.publish);
        END IF;
    END LOOP;
    FOR r IN SELECT l.pubname, l.tbl, l.cols FROM public.scd_publication_lists() l WHERE l.node = side ORDER BY 1, 2 LOOP
        CONTINUE WHEN to_regclass('public.' || quote_ident(r.tbl)) IS NULL;
        want := public.scd_publication_columns(r.tbl, r.cols);
        SELECT array_agg(c::text ORDER BY c::text COLLATE "C") INTO have
        FROM pg_publication_tables t, unnest(t.attnames) AS c
        WHERE t.pubname = r.pubname AND t.schemaname = 'public' AND t.tablename = r.tbl;
        CONTINUE WHEN have = want;
        IF have IS NOT NULL THEN
            EXECUTE format('ALTER PUBLICATION %I DROP TABLE public.%I', r.pubname, r.tbl);
        END IF;
        EXECUTE format('ALTER PUBLICATION %I ADD TABLE public.%I (%s)', r.pubname, r.tbl,
            (SELECT string_agg(quote_ident(c), ', ' ORDER BY c COLLATE "C") FROM unnest(want) AS c));
        RETURN NEXT format('%s %s.%s (%s)', CASE WHEN have IS NULL THEN 'add' ELSE 'change' END, r.pubname, r.tbl,
            array_to_string(want, ', '));
    END LOOP;
    FOR r IN
        SELECT t.pubname::text AS pubname, t.tablename::text AS tbl
        FROM pg_publication_tables t
        WHERE t.pubname IN (SELECT DISTINCT l.pubname FROM public.scd_publication_lists() l WHERE l.node = side)
          AND NOT EXISTS (
              SELECT 1 FROM public.scd_publication_lists() l
              WHERE l.pubname = t.pubname AND l.tbl = t.tablename AND t.schemaname = 'public'
          )
        ORDER BY 1, 2
    LOOP
        EXECUTE format('ALTER PUBLICATION %I DROP TABLE public.%I', r.pubname, r.tbl);
        RETURN NEXT format('drop %s.%s', r.pubname, r.tbl);
    END LOOP;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_unpublish_column(tbl text, col text)
RETURNS void
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
BEGIN
    IF EXISTS (
        SELECT 1 FROM pg_attribute a
        WHERE a.attrelid = ('public.' || quote_ident(tbl))::regclass AND a.attname = col AND a.attnotnull AND NOT a.atthasdef
    ) THEN
        RAISE EXCEPTION 'scd_unpublish_column: %.% is NOT NULL without a default; subscribers would reject inserts', tbl, col
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    INSERT INTO public.scd_unpublished_columns (tbl, col) VALUES (tbl, col) ON CONFLICT DO NOTHING;
    IF EXISTS (SELECT 1 FROM pg_publication p WHERE p.pubname IN (SELECT l.pubname FROM public.scd_publication_lists() l)) THEN
        PERFORM public.scd_sync_publications();
    END IF;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_configure(node text, role text, mode text)
RETURNS SETOF text
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    s record;
    stored text;
BEGIN
    IF node NOT IN ('core', 'main', 'star') THEN
        RAISE EXCEPTION 'scd_configure: node must be core, main or star, got %', node USING ERRCODE = 'invalid_parameter_value';
    END IF;
    IF role NOT IN ('core', 'api') OR (role = 'core') <> (node = 'core') THEN
        RAISE EXCEPTION 'scd_configure: role must be core on node core and api elsewhere, got % on %', role, node
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    IF mode NOT IN ('legacy', 'split') THEN
        RAISE EXCEPTION 'scd_configure: mode must be legacy or split, got %', mode USING ERRCODE = 'invalid_parameter_value';
    END IF;
    FOR s IN SELECT * FROM (VALUES ('scd.node', node), ('scd.role', role), ('scd.mode', mode)) AS v (name, value) LOOP
        SELECT substr(c, length(s.name) + 2) INTO stored
        FROM pg_db_role_setting d, unnest(d.setconfig) AS c
        WHERE d.setrole = 0
          AND d.setdatabase = (SELECT oid FROM pg_database WHERE datname = current_database())
          AND c LIKE s.name || '=%';
        IF stored IS DISTINCT FROM s.value THEN
            EXECUTE format('ALTER DATABASE %I SET %s = %L', current_database(), s.name, s.value);
            RETURN NEXT format('%s = %s', s.name, s.value);
        END IF;
        PERFORM set_config(s.name, s.value, false);
    END LOOP;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_grant_repl()
RETURNS SETOF text
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'scd_repl') THEN
        RETURN;
    END IF;
    GRANT USAGE ON SCHEMA public TO scd_repl;
    GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO scd_repl;
    GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO scd_repl;
    ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO scd_repl;
    ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT USAGE, SELECT, UPDATE ON SEQUENCES TO scd_repl;
    RETURN NEXT 'grant tables and sequences to scd_repl';
    IF NOT has_parameter_privilege('scd_repl', 'session_replication_role', 'SET') THEN
        BEGIN
            GRANT SET ON PARAMETER session_replication_role TO scd_repl;
        EXCEPTION WHEN insufficient_privilege THEN
            NULL;
        END;
        RETURN NEXT CASE
            WHEN has_parameter_privilege('scd_repl', 'session_replication_role', 'SET')
                THEN 'grant set on parameter session_replication_role to scd_repl'
            ELSE format('skip set on parameter session_replication_role: %s may not grant it', current_user)
        END;
    END IF;
    IF (SELECT p.proowner::regrole::text FROM pg_proc p WHERE p.oid = 'public.scd_dispatch()'::regprocedure) <> 'scd_repl' THEN
        BEGIN
            ALTER FUNCTION public.scd_dispatch() OWNER TO scd_repl;
            RETURN NEXT 'alter function scd_dispatch owner to scd_repl';
        EXCEPTION WHEN insufficient_privilege THEN
            RETURN NEXT format('skip scd_dispatch owner: %s may not set role scd_repl', current_user);
        END;
    END IF;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_repl_problems()
RETURNS SETOF text
LANGUAGE plpgsql
STABLE
SET search_path = public, pg_catalog
AS $f$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'scd_repl') THEN
        RETURN NEXT 'role scd_repl does not exist';
        RETURN;
    END IF;
    IF NOT has_parameter_privilege('scd_repl', 'session_replication_role', 'SET') THEN
        RETURN NEXT 'scd_repl lacks SET on parameter session_replication_role';
    END IF;
    IF NOT has_schema_privilege('scd_repl', 'public', 'USAGE') THEN
        RETURN NEXT 'scd_repl lacks USAGE on schema public';
    END IF;
    RETURN QUERY
        SELECT format('scd_repl lacks SELECT, INSERT, UPDATE, DELETE on table %s', c.relname)
        FROM pg_class c
        WHERE c.relnamespace = 'public'::regnamespace AND c.relkind IN ('r', 'p')
          AND NOT (has_table_privilege('scd_repl', c.oid, 'SELECT') AND has_table_privilege('scd_repl', c.oid, 'INSERT')
              AND has_table_privilege('scd_repl', c.oid, 'UPDATE') AND has_table_privilege('scd_repl', c.oid, 'DELETE'))
        ORDER BY c.relname;
    RETURN QUERY
        SELECT format('scd_repl lacks USAGE, SELECT, UPDATE on sequence %s', c.relname)
        FROM pg_class c
        WHERE c.relnamespace = 'public'::regnamespace AND c.relkind = 'S'
          AND NOT (has_sequence_privilege('scd_repl', c.oid, 'USAGE') AND has_sequence_privilege('scd_repl', c.oid, 'SELECT')
              AND has_sequence_privilege('scd_repl', c.oid, 'UPDATE'))
        ORDER BY c.relname;
    RETURN QUERY
        SELECT format('scd_dispatch is owned by %s and security definer is %s', p.proowner::regrole, p.prosecdef)
        FROM pg_proc p
        WHERE p.oid = 'public.scd_dispatch()'::regprocedure
          AND (p.proowner::regrole::text <> 'scd_repl' OR NOT p.prosecdef);
END
$f$;

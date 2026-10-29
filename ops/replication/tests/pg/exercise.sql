SET search_path = '';
SET client_min_messages = notice;
SET scd.mode = 'split';
SET scd.node = 'main';

DO $d$
DECLARE
    f regprocedure;
    chk record;
BEGIN
    FOR chk IN
        SELECT k.conrelid::regclass AS rel, k.conname
        FROM pg_catalog.pg_constraint k JOIN pg_catalog.pg_namespace n ON n.oid = k.connamespace
        WHERE n.nspname = 'public' AND k.contype = 'c'
        ORDER BY 1, 2
    LOOP
        EXECUTE format('ALTER TABLE %s DROP CONSTRAINT %I', chk.rel, chk.conname);
    END LOOP;
    EXECUTE (SELECT 'TRUNCATE ' || string_agg(format('public.%I', relname), ', ' ORDER BY relname)
             FROM pg_catalog.pg_class WHERE relnamespace = 'public'::regnamespace AND relkind = 'r');
    FOR f IN
        SELECT DISTINCT t.tgfoid::regprocedure
        FROM pg_catalog.pg_trigger t
        JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid
        JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public' AND NOT t.tgisinternal AND t.tgname NOT LIKE 'zz%'
    LOOP
        EXECUTE format('ALTER FUNCTION %s SET search_path = public', f);
    END LOOP;
END
$d$;

CREATE FUNCTION pg_temp.val(t oid, typmod int) RETURNS text LANGUAGE plpgsql AS $f$
DECLARE
    ty record;
    tname text := pg_catalog.format_type(t, typmod);
BEGIN
    SELECT typname, typtype, typcategory INTO ty FROM pg_catalog.pg_type WHERE oid = t;
    IF ty.typtype = 'e' THEN
        RETURN pg_catalog.quote_literal((SELECT enumlabel FROM pg_catalog.pg_enum WHERE enumtypid = t ORDER BY enumsortorder LIMIT 1)) || '::' || tname;
    END IF;
    RETURN CASE
        WHEN ty.typcategory = 'A' THEN '''{}''::' || tname
        WHEN ty.typname = 'bool' THEN 'false'
        WHEN ty.typcategory = 'N' THEN '(1 + floor(random() * 30000))::' || tname
        WHEN ty.typcategory = 'S' THEN 'left(md5(random()::text), 8)::' || tname
        WHEN ty.typcategory = 'D' THEN 'now()::' || tname
        WHEN ty.typcategory = 'T' THEN '''1 hour''::' || tname
        WHEN ty.typname = 'uuid' THEN 'gen_random_uuid()'
        WHEN ty.typname IN ('json', 'jsonb') THEN '''{}''::' || tname
        WHEN ty.typname = 'bytea' THEN '''\x00''::bytea'
        WHEN ty.typname = 'inet' THEN '''127.0.0.1''::inet'
        WHEN ty.typname = 'tsvector' THEN '''''::tsvector'
        ELSE NULL
    END;
END
$f$;

CREATE TEMP TABLE exercised (tbl text, role text, step text, outcome text);

CREATE FUNCTION pg_temp.step(tbl text, role text, name text, sql text, j jsonb DEFAULT NULL) RETURNS jsonb LANGUAGE plpgsql AS $f$
DECLARE
    out jsonb;
BEGIN
    EXECUTE sql INTO out USING j;
    INSERT INTO pg_temp.exercised VALUES (tbl, role, name, 'ran');
    RETURN out;
EXCEPTION WHEN data_exception OR integrity_constraint_violation THEN
    INSERT INTO pg_temp.exercised VALUES (tbl, role, name, 'data: ' || SQLSTATE);
    RETURN NULL;
END
$f$;

DO $d$
DECLARE
    t record;
    node_role text;
    done jsonb;
    msg text;
    det text;
    ins_cols text;
    ins_vals text;
    ins_new text;
    all_cols text;
    pk_cols text;
    first_col text;
    versions text[];
    j jsonb;
    bumped jsonb;
    pk jsonb;
    rel text;
    from_j text;
    where_pk text;
BEGIN
    FOR t IN
        SELECT c.oid, c.relname::text AS name
        FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public' AND c.relkind = 'r'
          AND (EXISTS (SELECT 1 FROM pg_catalog.pg_trigger g WHERE g.tgrelid = c.oid AND g.tgname LIKE 'zz%')
               OR EXISTS (SELECT 1 FROM pg_catalog.pg_proc p WHERE p.proname = 'scd_purge_' || c.relname))
        ORDER BY 2
    LOOP
        rel := 'public.' || quote_ident(t.name);
        SELECT string_agg(quote_ident(a.attname), ', ' ORDER BY a.attnum),
               string_agg(pg_temp.val(a.atttypid, a.atttypmod), ', ' ORDER BY a.attnum)
        INTO ins_cols, ins_vals
        FROM pg_catalog.pg_attribute a
        LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
        WHERE a.attrelid = t.oid AND a.attnum > 0 AND NOT a.attisdropped AND a.attgenerated = '' AND a.attnotnull AND d.adbin IS NULL;
        IF ins_vals IS NULL AND ins_cols IS NOT NULL THEN
            INSERT INTO pg_temp.exercised VALUES (t.name, '-', 'all', 'unsupported column type');
            CONTINUE;
        END IF;
        ins_new := CASE WHEN ins_cols IS NULL THEN 'DEFAULT VALUES' ELSE format('(%s) VALUES (%s)', ins_cols, ins_vals) END;
        SELECT string_agg(quote_ident(a.attname), ', ' ORDER BY a.attnum),
               (array_agg(quote_ident(a.attname) ORDER BY a.attnum) FILTER (WHERE NOT a.attnum = ANY (i.indkey)))[1],
               array_agg(a.attname::text ORDER BY a.attnum) FILTER (WHERE a.attname ~ '(^|_)v$' AND a.atttypid = 'bigint'::regtype)
        INTO all_cols, first_col, versions
        FROM pg_catalog.pg_attribute a
        LEFT JOIN pg_catalog.pg_index i ON i.indrelid = a.attrelid AND i.indisprimary
        WHERE a.attrelid = t.oid AND a.attnum > 0 AND NOT a.attisdropped AND a.attgenerated = '';
        SELECT string_agg(quote_ident(a.attname), ', ' ORDER BY k.ord)
        INTO pk_cols
        FROM pg_catalog.pg_index i
        CROSS JOIN LATERAL unnest(i.indkey::int2[]) WITH ORDINALITY AS k(attnum, ord)
        JOIN pg_catalog.pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum
        WHERE i.indrelid = t.oid AND i.indisprimary;
        from_j := format('SELECT %s FROM jsonb_populate_record(NULL::%s, $1)', all_cols, rel);
        where_pk := format('(%s) = (SELECT %s FROM jsonb_populate_record(NULL::%s, $1))', pk_cols, pk_cols, rel);
        FOREACH node_role IN ARRAY ARRAY['api', 'core'] LOOP
            BEGIN
                PERFORM set_config('scd.role', node_role, true);
                PERFORM set_config('session_replication_role', 'replica', true);
                j := pg_temp.step(t.name, node_role, 'replica insert', format('INSERT INTO %s %s RETURNING to_jsonb(%s.*)', rel, ins_new, quote_ident(t.name)));
                IF j IS NULL THEN
                    SELECT coalesce(jsonb_agg(to_jsonb(e)), '[]') INTO done FROM pg_temp.exercised e WHERE e.tbl = t.name AND e.role = node_role;
                    RAISE EXCEPTION 'scd_exercise_done' USING DETAIL = done::text;
                END IF;
                PERFORM pg_temp.step(t.name, node_role, 'replica insert, same key', format('INSERT INTO %s (%s) %s RETURNING NULL::jsonb', rel, all_cols, from_j), j);
                IF versions IS NOT NULL THEN
                    SELECT j || jsonb_object_agg(v, 4000000000000000000) INTO bumped FROM unnest(versions) v;
                    PERFORM pg_temp.step(t.name, node_role, 'replica insert, newer versions', format('INSERT INTO %s (%s) %s RETURNING NULL::jsonb', rel, all_cols, from_j), bumped);
                END IF;
                IF first_col IS NOT NULL THEN
                    PERFORM pg_temp.step(t.name, node_role, 'replica update', format('UPDATE %s SET %s = %s WHERE %s RETURNING NULL::jsonb', rel, first_col, first_col, where_pk), j);
                    PERFORM set_config('session_replication_role', 'origin', true);
                    PERFORM pg_temp.step(t.name, node_role, 'origin update', format('UPDATE %s SET %s = %s WHERE %s RETURNING NULL::jsonb', rel, first_col, first_col, where_pk), j);
                END IF;
                PERFORM set_config('session_replication_role', 'origin', true);
                PERFORM pg_temp.step(t.name, node_role, 'origin insert', format('INSERT INTO %s %s RETURNING NULL::jsonb', rel, ins_new));
                PERFORM set_config('session_replication_role', 'replica', true);
                IF EXISTS (SELECT 1 FROM pg_catalog.pg_proc WHERE proname = 'scd_purge_' || t.name) THEN
                    SELECT jsonb_object_agg(key, value) INTO pk FROM jsonb_each(j) WHERE quote_ident(key) = ANY (string_to_array(pk_cols, ', '));
                    PERFORM pg_temp.step(t.name, node_role, 'purge', format('SELECT NULL::jsonb FROM (SELECT public.scd_purge(%L, $1)) s', t.name), pk);
                    PERFORM pg_temp.step(t.name, node_role, 'replica insert after purge', format('INSERT INTO %s (%s) %s RETURNING NULL::jsonb', rel, all_cols, from_j), j);
                END IF;
                PERFORM pg_temp.step(t.name, node_role, 'replica delete', format('DELETE FROM %s WHERE %s RETURNING NULL::jsonb', rel, where_pk), j);
                PERFORM pg_temp.step(t.name, node_role, 'replica insert again', format('INSERT INTO %s (%s) %s RETURNING NULL::jsonb', rel, all_cols, from_j), j);
                PERFORM set_config('session_replication_role', 'origin', true);
                PERFORM pg_temp.step(t.name, node_role, 'origin delete', format('DELETE FROM %s WHERE %s RETURNING NULL::jsonb', rel, where_pk), j);
                SELECT coalesce(jsonb_agg(to_jsonb(e)), '[]') INTO done FROM pg_temp.exercised e WHERE e.tbl = t.name AND e.role = node_role;
                RAISE EXCEPTION 'scd_exercise_done' USING DETAIL = done::text;
            EXCEPTION WHEN raise_exception THEN
                GET STACKED DIAGNOSTICS msg = MESSAGE_TEXT, det = PG_EXCEPTION_DETAIL;
                IF msg <> 'scd_exercise_done' THEN
                    RAISE;
                END IF;
                INSERT INTO pg_temp.exercised SELECT * FROM jsonb_populate_recordset(NULL::pg_temp.exercised, det::jsonb);
            END;
        END LOOP;
    END LOOP;
END
$d$;

DO $d$
DECLARE
    r record;
BEGIN
    FOR r IN SELECT outcome, count(*) AS n, count(DISTINCT tbl) AS tables FROM pg_temp.exercised GROUP BY 1 ORDER BY 1 LOOP
        RAISE NOTICE 'ok exercise: % steps % on % tables', r.n, r.outcome, r.tables;
    END LOOP;
    FOR r IN SELECT tbl, string_agg(DISTINCT role || ' ' || outcome, ', ') AS why FROM pg_temp.exercised WHERE step = 'replica insert' AND outcome <> 'ran' GROUP BY 1 ORDER BY 1 LOOP
        RAISE NOTICE 'ok exercise: no row for %: %', r.tbl, r.why;
    END LOOP;
END
$d$;

SELECT 'exercise ok' AS result;

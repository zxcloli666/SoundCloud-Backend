SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_verify_tables()
RETURNS SETOF text
LANGUAGE sql STABLE
SET search_path = public, pg_catalog
AS $f$
SELECT l.tbl FROM public.scd_publication_lists() l
WHERE l.pubname = 'scd_core_pub' AND l.tbl <> 'repl_tombstones'
  AND public.scd_table_class(l.tbl) IN ('C', 'M', 'A')
  AND to_regclass('public.' || quote_ident(l.tbl)) IS NOT NULL
ORDER BY l.tbl COLLATE "C"
$f$;

CREATE OR REPLACE FUNCTION public.scd_verify_shown(rel text)
RETURNS text[]
LANGUAGE sql STABLE
SET search_path = public, pg_catalog
AS $f$
SELECT coalesce(array_agg(c ORDER BY c COLLATE "C"), '{}')
FROM unnest(public.scd_publication_columns(rel,
    (SELECT l.cols FROM public.scd_publication_lists() l WHERE l.pubname = 'scd_core_pub' AND l.tbl = rel))) c
WHERE NOT EXISTS (SELECT 1 FROM public.scd_core_columns() cc WHERE cc.tbl = rel AND cc.col = c)
$f$;

CREATE OR REPLACE FUNCTION public.scd_verify_parts(rel text, shown text[], OUT keycols text, OUT keyobj text, OUT above text, OUT upto text, OUT same text, OUT hidden text[])
LANGUAGE plpgsql STABLE
SET search_path = public, pg_catalog
AS $f$
DECLARE
    bound text;
BEGIN
    SELECT string_agg(format('t.%I', a.attname), ', ' ORDER BY k.n),
           'jsonb_build_object(' || string_agg(format('%L, t.%I', a.attname, a.attname), ', ' ORDER BY k.n) || ')',
           string_agg(format('(($B ->> %L))::%s', a.attname, format_type(a.atttypid, a.atttypmod))
                      || CASE WHEN a.attcollation <> 0 THEN format(' COLLATE %I.%I', cn.nspname, co.collname) ELSE '' END, ', ' ORDER BY k.n),
           string_agg(format('t.%I = r.%I', a.attname, a.attname), ' AND ' ORDER BY k.n)
    INTO keycols, keyobj, bound, same
    FROM pg_index i
    CROSS JOIN LATERAL unnest(i.indkey::int2[]) WITH ORDINALITY AS k (attnum, n)
    JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum
    LEFT JOIN pg_collation co ON co.oid = a.attcollation
    LEFT JOIN pg_namespace cn ON cn.oid = co.collnamespace
    WHERE i.indrelid = ('public.' || quote_ident(rel))::regclass AND i.indisprimary;
    IF keycols IS NULL THEN
        RAISE EXCEPTION 'scd_verify: public.% has no primary key', rel;
    END IF;
    above := format('(%s) > (%s)', keycols, replace(bound, '$B', '$1'));
    upto := format('(%s) <= (%s)', keycols, replace(bound, '$B', '$2'));
    SELECT coalesce(array_agg(a.attname::text), '{}') INTO hidden
    FROM pg_attribute a
    WHERE a.attrelid = ('public.' || quote_ident(rel))::regclass AND a.attnum > 0 AND NOT a.attisdropped
      AND a.attname::text <> ALL (shown);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_verify_chunk(rel text, lo jsonb, hi jsonb, shown text[] DEFAULT NULL, OUT n bigint, OUT h text, OUT keys jsonb)
LANGUAGE plpgsql STABLE
SET search_path = public, pg_catalog
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
DECLARE
    p record;
BEGIN
    SELECT * INTO p FROM public.scd_verify_parts(rel, coalesce(shown, public.scd_verify_shown(rel)));
    EXECUTE format(
        'SELECT count(*), coalesce(md5(string_agg(q.h, %L ORDER BY q.n)), %L), coalesce(jsonb_object_agg(q.k, q.h), %L)
         FROM (SELECT %s::text AS k, md5((pg_catalog.to_jsonb(t) - $3)::text) AS h, row_number() OVER (ORDER BY %s) AS n
               FROM public.%I t WHERE %s AND %s) q',
        '', '', '{}', p.keyobj, p.keycols, rel,
        CASE WHEN lo IS NULL THEN 'true' ELSE p.above END,
        CASE WHEN hi IS NULL THEN 'true' ELSE p.upto END)
    INTO n, h, keys
    USING lo, hi, p.hidden;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_verify_next(rel text, lo jsonb, size integer)
RETURNS jsonb
LANGUAGE plpgsql STABLE
SET search_path = public, pg_catalog
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
DECLARE
    p record;
    hi jsonb;
BEGIN
    SELECT * INTO p FROM public.scd_verify_parts(rel, '{}');
    EXECUTE format('SELECT %s FROM public.%I t WHERE %s ORDER BY %s OFFSET $3 LIMIT 1',
        p.keyobj, rel, CASE WHEN lo IS NULL THEN 'true' ELSE p.above END, p.keycols)
    INTO hi
    USING lo, NULL::jsonb, greatest(size, 1) - 1;
    RETURN hi;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_repair_send(rel text, k jsonb, body jsonb)
RETURNS void
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    rid uuid := gen_random_uuid();
BEGIN
    INSERT INTO public.repl_repair (id, src_node, tbl, pk, "row") VALUES (rid, public.scd_node(), rel, k, body);
    DELETE FROM public.repl_repair x WHERE x.id = rid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_verify_send(rel text, k jsonb, op jsonb)
RETURNS void
LANGUAGE sql
SET search_path = public, pg_catalog
AS $f$
SELECT public.scd_repair_send('repl_checksums', jsonb_build_object('tbl', rel, 'key', k), op)
$f$;

CREATE OR REPLACE FUNCTION public.scd_verify_probe(budget integer DEFAULT 10, size integer DEFAULT 500, OUT state text, OUT probes integer, OUT rel text, OUT wrapped boolean)
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    cursor_at jsonb;
    walk text;
    shown text[];
    lo jsonb;
    hi jsonb;
    c record;
BEGIN
    probes := 0;
    wrapped := false;
    IF public.scd_mode() <> 'split' OR public.scd_role() <> 'core' THEN
        state := 'idle';
        RETURN;
    END IF;
    IF NOT pg_try_advisory_xact_lock(hashtextextended('scd.verify', 0)) THEN
        state := 'busy';
        RETURN;
    END IF;
    state := 'ran';
    SELECT x.hash::jsonb INTO cursor_at FROM public.repl_checksums x WHERE x.node = 'core' AND x.tbl = '*' AND x.range = 'cursor';
    walk := (SELECT v FROM public.scd_verify_tables() v WHERE v COLLATE "C" >= coalesce(cursor_at ->> 'tbl', '') COLLATE "C" ORDER BY v COLLATE "C" LIMIT 1);
    IF walk = cursor_at ->> 'tbl' THEN
        lo := nullif(cursor_at -> 'lo', 'null');
    END IF;
    IF walk IS NULL THEN
        walk := (SELECT v FROM public.scd_verify_tables() v LIMIT 1);
    END IF;
    WHILE walk IS NOT NULL AND probes < budget LOOP
        IF rel IS DISTINCT FROM walk THEN
            shown := public.scd_verify_shown(walk);
        END IF;
        rel := walk;
        hi := public.scd_verify_next(walk, lo, size);
        SELECT * INTO c FROM public.scd_verify_chunk(walk, lo, hi, shown);
        PERFORM public.scd_verify_send(walk, jsonb_build_object('lo', lo, 'hi', hi),
            jsonb_build_object('scd_op', 'probe', 'n', c.n, 'h', c.h, 'attempt', 0, 'cols', to_jsonb(shown)));
        probes := probes + 1;
        IF hi IS NOT NULL THEN
            lo := hi;
            CONTINUE;
        END IF;
        lo := NULL;
        walk := (SELECT v FROM public.scd_verify_tables() v WHERE v COLLATE "C" > walk COLLATE "C" ORDER BY v COLLATE "C" LIMIT 1);
        IF walk IS NULL THEN
            wrapped := true;
        END IF;
    END LOOP;
    INSERT INTO public.repl_checksums (node, tbl, range, hash, at)
    VALUES ('core', '*', 'cursor', jsonb_build_object('tbl', walk, 'lo', lo)::text, now())
    ON CONFLICT (node, tbl, range) DO UPDATE SET hash = EXCLUDED.hash, at = EXCLUDED.at;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_verify_answer(probe uuid, rel text, k jsonb, op jsonb)
RETURNS boolean
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    c record;
    diff jsonb;
BEGIN
    IF EXISTS (SELECT 1 FROM public.repl_resend x WHERE (CASE WHEN x.tbl = 'repl_tombstones' THEN x.pk ->> 'tbl' ELSE x.tbl END) = rel) THEN
        IF current_setting('session_replication_role') = 'replica' THEN
            INSERT INTO public.repl_checksums (node, tbl, range, hash, at)
            VALUES (public.scd_node(), rel, 'wait ' || probe::text, jsonb_build_object('key', k, 'op', op)::text, now())
            ON CONFLICT DO NOTHING;
        END IF;
        RETURN false;
    END IF;
    SELECT * INTO c FROM public.scd_verify_chunk(rel, nullif(k -> 'lo', 'null'), nullif(k -> 'hi', 'null'),
        ARRAY(SELECT jsonb_array_elements_text(op -> 'cols')));
    IF jsonb_typeof(op -> 'keys') = 'object' THEN
        diff := (SELECT jsonb_object_agg(coalesce(m.key, t.key), m.value)
                 FROM jsonb_each_text(c.keys) m
                 FULL JOIN jsonb_each_text(op -> 'keys') t ON t.key = m.key
                 WHERE m.value IS DISTINCT FROM t.value);
        IF diff IS NULL THEN
            RETURN true;
        END IF;
    ELSIF c.n = (op ->> 'n')::bigint AND c.h = op ->> 'h' THEN
        RETURN true;
    END IF;
    INSERT INTO public.repl_checksums (node, tbl, range, hash, at)
    VALUES (public.scd_node(), rel, probe::text,
        jsonb_build_object('lo', k -> 'lo', 'hi', k -> 'hi', 'attempt', op -> 'attempt', 'n', c.n, 'keys', diff)::text, now())
    ON CONFLICT DO NOTHING;
    IF current_setting('session_replication_role') = 'replica' THEN
        INSERT INTO public.repl_resend (tbl, pk)
        VALUES ('repl_checksums', jsonb_build_object('node', public.scd_node(), 'tbl', rel, 'range', probe::text))
        ON CONFLICT DO NOTHING;
    END IF;
    RETURN true;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_verify_drop(rel text, k jsonb, receipts jsonb)
RETURNS void
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    cls text := public.scd_table_class(rel);
BEGIN
    IF cls = 'C' THEN
        PERFORM public.scd_lock_row(rel, k);
        IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = rel AND p.pk = k) THEN
            PERFORM public.scd_purge(rel, k);
        END IF;
        RETURN;
    END IF;
    IF cls = 'M' THEN
        IF jsonb_typeof(receipts) IS DISTINCT FROM 'array' THEN
            RETURN;
        END IF;
        INSERT INTO public.repl_purges (tbl, pk, purged_v, at)
        SELECT rel, k, (r ->> 'purged_v')::bigint, (r ->> 'at')::timestamptz FROM jsonb_array_elements(receipts) r
        ON CONFLICT DO NOTHING;
    END IF;
    IF cls IN ('M', 'A') THEN
        EXECUTE format('DELETE FROM public.%I t USING jsonb_populate_record(NULL::public.%I, $1) r WHERE %s',
            rel, rel, (SELECT p.same FROM public.scd_verify_parts(rel, '{}') p))
        USING k;
    END IF;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_repair_op(id uuid, rel text, k jsonb, op jsonb)
RETURNS void
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
BEGIN
    CASE op ->> 'scd_op'
        WHEN 'probe' THEN
            PERFORM public.scd_verify_answer(id, rel, k, op);
        WHEN 'drop' THEN
            PERFORM public.scd_verify_drop(rel, k, op -> 'receipts');
        WHEN 'resend' THEN
            IF op ->> 'node' = public.scd_node() THEN
                INSERT INTO public.repl_resend (tbl, pk) VALUES (rel, k) ON CONFLICT DO NOTHING;
            END IF;
        ELSE
            NULL;
    END CASE;
EXCEPTION WHEN OTHERS THEN
    RAISE WARNING 'scd_repair_op % on public.% %: %', op ->> 'scd_op', rel, k, SQLERRM;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_on_repl_repair()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    cls text := public.scd_table_class(NEW.tbl);
BEGIN
    IF NEW.src_node = public.scd_node() THEN
        RETURN NULL;
    END IF;
    IF NEW."row" ? 'scd_op' THEN
        IF public.scd_role() = 'api' AND NEW.src_node = 'core' THEN
            PERFORM public.scd_repair_op(NEW.id, NEW.pk ->> 'tbl', NEW.pk -> 'key', NEW."row");
        END IF;
        RETURN NULL;
    END IF;
    IF (public.scd_role() = 'core' AND cls IN ('M', 'A', 'K')) OR (public.scd_role() = 'api' AND cls IN ('C', 'M', 'A')) THEN
        PERFORM public.scd_apply_row(NEW.tbl, NEW."row");
    END IF;
    RETURN NULL;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_verify_repair(dry boolean DEFAULT false, answers integer DEFAULT 50)
RETURNS TABLE (peer text, rel text, k jsonb, action text, attempt integer)
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    a record;
    body jsonb;
    report jsonb;
    r jsonb;
BEGIN
    IF public.scd_mode() <> 'split' OR public.scd_role() <> 'core' THEN
        RETURN;
    END IF;
    IF NOT pg_try_advisory_xact_lock(hashtextextended('scd.verify', 0)) THEN
        RETURN;
    END IF;
    FOR a IN
        SELECT x.node, x.tbl, x.range, x.hash FROM public.repl_checksums x
        WHERE x.node <> 'core' AND x.range NOT LIKE 'wait %'
        ORDER BY x.at
        LIMIT answers
        FOR UPDATE SKIP LOCKED
    LOOP
        BEGIN
            body := a.hash::jsonb;
            report := public.scd_verify_settle(a.node, a.tbl, body, dry);
        EXCEPTION WHEN OTHERS THEN
            report := jsonb_build_array(jsonb_build_object('k', NULL, 'action', 'error: ' || SQLERRM, 'attempt', 0));
        END;
        DELETE FROM public.repl_checksums x WHERE x.node = a.node AND x.tbl = a.tbl AND x.range = a.range;
        FOR r IN SELECT e FROM jsonb_array_elements(report) e LOOP
            peer := a.node;
            rel := a.tbl;
            k := r -> 'k';
            action := r ->> 'action';
            attempt := (r ->> 'attempt')::integer;
            RETURN NEXT;
        END LOOP;
    END LOOP;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_verify_settle(peer text, rel text, body jsonb, dry boolean)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    cls text := public.scd_table_class(rel);
    tombs boolean := EXISTS (SELECT 1 FROM pg_catalog.pg_trigger g WHERE g.tgrelid = ('public.' || quote_ident(rel))::regclass AND g.tgname = 'zz_tomb');
    lo jsonb := nullif(body -> 'lo', 'null');
    hi jsonb := nullif(body -> 'hi', 'null');
    attempt integer := coalesce((body ->> 'attempt')::integer, 0);
    mine record;
    d record;
    k jsonb;
    image jsonb;
    receipts jsonb;
    tomb jsonb;
    acts text[];
    act text;
    report jsonb := '[]';
BEGIN
    SELECT * INTO mine FROM public.scd_verify_chunk(rel, lo, hi);
    IF jsonb_typeof(body -> 'keys') IS DISTINCT FROM 'object' THEN
        PERFORM public.scd_verify_send(rel, jsonb_build_object('lo', lo, 'hi', hi),
            jsonb_build_object('scd_op', 'probe', 'attempt', 1, 'cols', to_jsonb(public.scd_verify_shown(rel)), 'keys', mine.keys));
        RETURN jsonb_build_array(jsonb_build_object('k', jsonb_build_object('lo', lo, 'hi', hi), 'action', 'recheck', 'attempt', 0));
    END IF;
    FOR d IN
        SELECT p.key, mine.keys ->> p.key AS ours, p.value AS theirs
        FROM jsonb_each_text(body -> 'keys') p
        WHERE (mine.keys ->> p.key) IS DISTINCT FROM p.value
        ORDER BY 1
    LOOP
        k := d.key::jsonb;
        IF d.ours IS NOT NULL THEN
            acts := CASE WHEN cls <> 'C' AND d.theirs IS NOT NULL THEN ARRAY['row', 'resend'] ELSE ARRAY['row'] END;
        ELSIF cls = 'C' OR (cls = 'A' AND NOT tombs) THEN
            acts := ARRAY['drop'];
        ELSIF cls = 'M' THEN
            receipts := (SELECT jsonb_agg(jsonb_build_object('purged_v', x.purged_v, 'at', x.at) ORDER BY x.at)
                         FROM public.repl_purges x WHERE x.tbl = rel AND x.pk = k);
            acts := CASE WHEN receipts IS NULL THEN ARRAY['resend'] ELSE ARRAY['drop'] END;
        ELSE
            tomb := (SELECT jsonb_build_object('tbl', x.tbl, 'pk', x.pk, 'v', x.v)
                     FROM public.repl_tombstones x WHERE x.tbl = rel AND x.pk = k ORDER BY x.v DESC LIMIT 1);
            acts := CASE WHEN tomb IS NULL THEN ARRAY['resend'] ELSE ARRAY['tomb'] END;
        END IF;
        FOREACH act IN ARRAY acts LOOP
            IF NOT dry THEN
                CASE act
                    WHEN 'row' THEN
                        image := public.scd_row_json(rel, k);
                        IF image IS NOT NULL THEN
                            PERFORM public.scd_repair_send(rel, k, image);
                        END IF;
                    WHEN 'resend' THEN
                        PERFORM public.scd_verify_send(rel, k, jsonb_build_object('scd_op', 'resend', 'node', peer));
                    WHEN 'drop' THEN
                        PERFORM public.scd_verify_send(rel, k, jsonb_build_object('scd_op', 'drop', 'receipts', receipts));
                    WHEN 'tomb' THEN
                        PERFORM public.scd_repair_send('repl_tombstones', tomb, public.scd_row_json('repl_tombstones', tomb));
                END CASE;
            END IF;
            report := report || jsonb_build_array(jsonb_build_object('k', k, 'action', act, 'attempt', attempt));
        END LOOP;
    END LOOP;
    IF jsonb_array_length(report) > 0 AND NOT dry AND attempt < 3 THEN
        PERFORM public.scd_verify_send(rel, jsonb_build_object('lo', lo, 'hi', hi),
            jsonb_build_object('scd_op', 'probe', 'attempt', attempt + 1, 'cols', to_jsonb(public.scd_verify_shown(rel)), 'keys', mine.keys));
    END IF;
    RETURN report;
END
$f$;

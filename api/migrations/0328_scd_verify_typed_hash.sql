SET LOCAL lock_timeout = '5s';

DROP FUNCTION IF EXISTS public.scd_verify_chunk(text, jsonb, jsonb, text[]);

CREATE OR REPLACE FUNCTION public.scd_verify_cols(rel text, shown text[])
RETURNS text
LANGUAGE sql STABLE
SET search_path = public, pg_catalog, pg_temp
AS $f$
SELECT string_agg(
           CASE WHEN NOT coalesce(co.collisdeterministic, true) THEN format('t.%I::text', c.col)
                WHEN a.atttypid = ANY (typed.oids) THEN format('t.%I', c.col)
                WHEN a.atttypid = 'jsonb'::regtype THEN format('t.%I, pg_catalog.jsonb_typeof(t.%I)', c.col, c.col)
                WHEN ty.typcategory = 'A' AND ty.typelem = ANY (typed.oids) THEN format('t.%I, pg_catalog.array_dims(t.%I)', c.col, c.col)
                ELSE format('t.%I::text', c.col) END, ', ' ORDER BY c.n)
FROM unnest(shown) WITH ORDINALITY AS c (col, n)
CROSS JOIN (SELECT ARRAY['text', 'character varying', 'smallint', 'integer', 'bigint', 'boolean', 'uuid', 'timestamp with time zone',
                         'timestamp without time zone', 'date', 'bytea', 'real', 'double precision']::regtype[]::oid[] AS oids) typed
JOIN pg_attribute a ON a.attrelid = ('public.' || quote_ident(rel))::regclass AND a.attname = c.col AND a.attnum > 0 AND NOT a.attisdropped
JOIN pg_type ty ON ty.oid = a.atttypid
LEFT JOIN pg_collation co ON co.oid = a.attcollation
$f$;

CREATE OR REPLACE FUNCTION public.scd_verify_chunk(rel text, lo jsonb, hi jsonb, shown text[] DEFAULT NULL, keyed boolean DEFAULT true, OUT n bigint, OUT h text, OUT keys jsonb)
LANGUAGE plpgsql STABLE
SET search_path = public, pg_catalog, pg_temp
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
DECLARE
    p record;
BEGIN
    shown := coalesce(shown, public.scd_verify_shown(rel));
    SELECT * INTO p FROM public.scd_verify_parts(rel, shown);
    EXECUTE format(
        'SELECT count(*), coalesce(md5(string_agg(x.h::text, %L ORDER BY %s)), %L), %s
         FROM public.%I t CROSS JOIN LATERAL (SELECT pg_catalog.hash_record_extended(ROW(%s), 1) AS h) x
         WHERE %s AND %s',
        ',', p.keycols, '', CASE WHEN keyed THEN format('coalesce(jsonb_object_agg(%s::text, x.h::text), %L)', p.keyobj, '{}') ELSE 'NULL::jsonb' END,
        rel, public.scd_verify_cols(rel, shown),
        CASE WHEN lo IS NULL THEN 'true' ELSE p.above END,
        CASE WHEN hi IS NULL THEN 'true' ELSE p.upto END)
    INTO n, h, keys
    USING lo, hi;
END
$f$;
CREATE OR REPLACE FUNCTION public.scd_verify_probe(budget integer DEFAULT 10, size integer DEFAULT 500, OUT state text, OUT probes integer, OUT rel text, OUT wrapped boolean)
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
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
        SELECT * INTO c FROM public.scd_verify_chunk(walk, lo, hi, shown, false);
        PERFORM public.scd_verify_send(walk, jsonb_build_object('lo', lo, 'hi', hi),
            jsonb_build_object('scd_op', 'probe', 'hv', 2, 'n', c.n, 'h', c.h, 'attempt', 0, 'cols', to_jsonb(shown)));
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
SET search_path = public, pg_catalog, pg_temp
AS $f$
DECLARE
    c record;
    diff jsonb;
    held jsonb;
BEGIN
    IF coalesce((op ->> 'hv')::integer, 1) <> 2 THEN
        RETURN true;
    END IF;
    IF EXISTS (SELECT 1 FROM public.repl_resend x WHERE (CASE WHEN x.tbl = 'repl_tombstones' THEN x.pk ->> 'tbl' ELSE x.tbl END) = rel) THEN
        IF current_setting('session_replication_role') = 'replica' THEN
            INSERT INTO public.repl_checksums (node, tbl, range, hash, at)
            VALUES (public.scd_node(), rel, 'wait ' || probe::text, jsonb_build_object('key', k, 'op', op)::text, now())
            ON CONFLICT DO NOTHING;
        END IF;
        RETURN false;
    END IF;
    SELECT * INTO c FROM public.scd_verify_chunk(rel, nullif(k -> 'lo', 'null'), nullif(k -> 'hi', 'null'),
        ARRAY(SELECT jsonb_array_elements_text(op -> 'cols')), jsonb_typeof(op -> 'keys') = 'object');
    IF jsonb_typeof(op -> 'keys') = 'object' THEN
        diff := (SELECT jsonb_object_agg(coalesce(m.key, t.key), m.value)
                 FROM jsonb_each_text(c.keys) m
                 FULL JOIN jsonb_each_text(op -> 'keys') t ON t.key = m.key
                 WHERE m.value IS DISTINCT FROM t.value);
        IF diff IS NULL THEN
            RETURN true;
        END IF;
        IF public.scd_table_class(rel) = 'M' THEN
            held := (SELECT jsonb_object_agg(d.key, r.list)
                     FROM jsonb_object_keys(diff) d (key)
                     CROSS JOIN LATERAL (
                         SELECT jsonb_agg(jsonb_build_object('purged_v', x.purged_v, 'at', x.at) ORDER BY x.at) AS list
                         FROM public.repl_purges x WHERE x.tbl = rel AND x.pk = d.key::jsonb
                     ) r
                     WHERE r.list IS NOT NULL);
        END IF;
    ELSIF c.n = (op ->> 'n')::bigint AND c.h = op ->> 'h' THEN
        RETURN true;
    END IF;
    INSERT INTO public.repl_checksums (node, tbl, range, hash, at)
    VALUES (public.scd_node(), rel, probe::text,
        jsonb_build_object('hv', 2, 'lo', k -> 'lo', 'hi', k -> 'hi', 'attempt', op -> 'attempt', 'n', c.n, 'keys', diff, 'receipts', held)::text, now())
    ON CONFLICT DO NOTHING;
    IF current_setting('session_replication_role') = 'replica' THEN
        INSERT INTO public.repl_resend (tbl, pk)
        VALUES ('repl_checksums', jsonb_build_object('node', public.scd_node(), 'tbl', rel, 'range', probe::text))
        ON CONFLICT DO NOTHING;
    END IF;
    RETURN true;
END
$f$;
CREATE OR REPLACE FUNCTION public.scd_verify_settle(peer text, rel text, body jsonb, dry boolean)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
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
    held jsonb;
    skew boolean;
    tomb jsonb;
    acts text[];
    act text;
    report jsonb := '[]';
BEGIN
    IF coalesce((body ->> 'hv')::integer, 1) <> 2 THEN
        RETURN jsonb_build_array(jsonb_build_object('k', jsonb_build_object('lo', lo, 'hi', hi), 'action', 'stale_hash', 'attempt', attempt));
    END IF;
    SELECT * INTO mine FROM public.scd_verify_chunk(rel, lo, hi);
    IF jsonb_typeof(body -> 'keys') IS DISTINCT FROM 'object' THEN
        PERFORM public.scd_verify_send(rel, jsonb_build_object('lo', lo, 'hi', hi),
            jsonb_build_object('scd_op', 'probe', 'hv', 2, 'attempt', 1, 'cols', to_jsonb(public.scd_verify_shown(rel)), 'keys', mine.keys));
        RETURN jsonb_build_array(jsonb_build_object('k', jsonb_build_object('lo', lo, 'hi', hi), 'action', 'recheck', 'attempt', 0));
    END IF;
    FOR d IN
        SELECT p.key, mine.keys ->> p.key AS ours, p.value AS theirs
        FROM jsonb_each_text(body -> 'keys') p
        WHERE (mine.keys ->> p.key) IS DISTINCT FROM p.value
        ORDER BY 1
    LOOP
        k := d.key::jsonb;
        receipts := NULL;
        skew := false;
        IF cls = 'M' THEN
            receipts := (SELECT jsonb_agg(jsonb_build_object('purged_v', x.purged_v, 'at', x.at) ORDER BY x.at)
                         FROM public.repl_purges x WHERE x.tbl = rel AND x.pk = k);
            held := CASE WHEN jsonb_typeof(body -> 'receipts' -> d.key) = 'array' THEN body -> 'receipts' -> d.key ELSE '[]' END;
            skew := EXISTS (
                SELECT 1 FROM (
                    SELECT (e ->> 'purged_v')::bigint AS v, (e ->> 'at')::timestamptz AS at, 'core' AS side
                    FROM jsonb_array_elements(coalesce(receipts, '[]')) e
                    UNION ALL
                    SELECT (e ->> 'purged_v')::bigint, (e ->> 'at')::timestamptz, 'peer'
                    FROM jsonb_array_elements(held) e
                ) s
                GROUP BY s.v, s.at
                HAVING count(DISTINCT s.side) = 1);
        END IF;
        IF d.ours IS NOT NULL THEN
            acts := CASE WHEN cls <> 'C' AND d.theirs IS NOT NULL THEN ARRAY['row', 'resend'] ELSE ARRAY['row'] END;
            IF skew THEN
                acts := ARRAY['receipts'] || acts;
            END IF;
        ELSIF cls = 'C' OR (cls = 'A' AND NOT tombs) THEN
            acts := ARRAY['drop'];
        ELSIF cls = 'M' THEN
            acts := CASE WHEN receipts IS NULL THEN ARRAY['resend'] ELSE ARRAY['drop'] END;
        ELSE
            tomb := (SELECT jsonb_build_object('tbl', x.tbl, 'pk', x.pk, 'v', x.v)
                     FROM public.repl_tombstones x WHERE x.tbl = rel AND x.pk = k ORDER BY x.v DESC LIMIT 1);
            acts := CASE WHEN tomb IS NULL THEN ARRAY['resend'] ELSE ARRAY['tomb'] END;
        END IF;
        FOREACH act IN ARRAY acts LOOP
            IF NOT dry THEN
                CASE act
                    WHEN 'receipts' THEN
                        PERFORM public.scd_verify_send(rel, k, jsonb_build_object('scd_op', 'receipts', 'receipts', coalesce(receipts, '[]')));
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
            jsonb_build_object('scd_op', 'probe', 'hv', 2, 'attempt', attempt + 1, 'cols', to_jsonb(public.scd_verify_shown(rel)), 'keys', mine.keys));
    END IF;
    RETURN report;
END
$f$;

SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_unique_race(nsp text, rel text, con text, detail text, snap pg_snapshot)
RETURNS xid
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    idx oid;
    keys text[];
    types text[];
    pred text;
    head text;
    tail text := ') already exists.';
    body text;
    vals text[];
    holder xid;
    ref bigint := pg_snapshot_xmax(snap)::text::bigint;
    lag bigint;
BEGIN
    SELECT i.indexrelid, pg_get_expr(i.indpred, i.indrelid) INTO idx, pred
    FROM pg_index i
    JOIN pg_class c ON c.oid = i.indexrelid
    JOIN pg_namespace n ON n.oid = c.relnamespace
    JOIN pg_class r ON r.oid = i.indrelid
    WHERE n.nspname = nsp AND c.relname = con AND r.relname = rel AND i.indisunique AND i.indexprs IS NULL;
    SELECT array_agg(quote_ident(a.attname) ORDER BY k.ord), array_agg(format_type(a.atttypid, a.atttypmod) ORDER BY k.ord)
    INTO keys, types
    FROM pg_index i
    CROSS JOIN LATERAL unnest(i.indkey::int2[]) WITH ORDINALITY AS k (attnum, ord)
    JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum
    WHERE i.indexrelid = idx AND k.ord <= i.indnkeyatts;
    head := 'Key (' || array_to_string(keys, ', ') || ')=(';
    IF keys IS NULL OR NOT coalesce(starts_with(detail, head) AND right(detail, length(tail)) = tail, false) THEN
        RETURN NULL;
    END IF;
    body := substr(detail, length(head) + 1, length(detail) - length(head) - length(tail));
    vals := CASE WHEN cardinality(keys) = 1 THEN ARRAY[body] ELSE string_to_array(body, ', ') END;
    IF cardinality(vals) <> cardinality(keys) THEN
        RETURN NULL;
    END IF;
    EXECUTE format('SELECT t.xmin FROM %I.%I t WHERE (%s) = (%s)%s', nsp, rel, array_to_string(keys, ', '),
        (SELECT string_agg(format('($1[%s])::%s', u.ord, u.ty), ', ' ORDER BY u.ord) FROM unnest(types) WITH ORDINALITY AS u (ty, ord)),
        coalesce(' AND (' || pred || ')', ''))
    INTO holder USING vals;
    IF holder IS NULL THEN
        RETURN NULL;
    END IF;
    lag := (holder::text::bigint - ref % 4294967296 + 4294967296) % 4294967296;
    IF lag >= 2147483648 THEN
        lag := lag - 4294967296;
    END IF;
    IF pg_visible_in_snapshot((ref + lag)::text::xid8, snap) THEN
        RETURN NULL;
    END IF;
    RETURN holder;
EXCEPTION WHEN OTHERS THEN
    RETURN NULL;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_xact_origin(x xid)
RETURNS oid
LANGUAGE plpgsql
STRICT
SET search_path = public, pg_catalog
AS $f$
BEGIN
    IF NOT current_setting('track_commit_timestamp')::boolean THEN
        RETURN NULL;
    END IF;
    RETURN (pg_xact_commit_timestamp_origin(x)).roident;
EXCEPTION WHEN OTHERS THEN
    RETURN NULL;
END
$f$;

DROP FUNCTION IF EXISTS public.scd_wt(text, jsonb);

CREATE FUNCTION public.scd_wt(kind text, args jsonb, OUT id uuid, OUT output jsonb)
LANGUAGE plpgsql
SET search_path = public, pg_catalog
SET timezone TO 'UTC'
AS $f$
DECLARE
    optimistic boolean;
    shipped boolean := public.scd_mode() = 'split' AND public.scd_role() = 'api';
    res jsonb;
    k jsonb;
    touched jsonb := '[]';
    snap pg_snapshot;
    nsp text;
    rel text;
    con text;
    msg text;
    det text;
BEGIN
    id := gen_random_uuid();
    SELECT x.optimistic INTO optimistic FROM public.scd_kinds() x WHERE x.kind = scd_wt.kind;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'scd_wt: unknown kind %', scd_wt.kind USING ERRCODE = 'invalid_parameter_value';
    END IF;
    IF optimistic OR NOT shipped THEN
        PERFORM set_config('scd.cmd', id::text, true);
        snap := pg_current_snapshot();
        BEGIN
            EXECUTE format('SELECT public.%I($1)', public.scd_kind_function(scd_wt.kind)) INTO res USING args;
        EXCEPTION WHEN unique_violation THEN
            GET STACKED DIAGNOSTICS nsp = SCHEMA_NAME, rel = TABLE_NAME, con = CONSTRAINT_NAME, msg = MESSAGE_TEXT, det = PG_EXCEPTION_DETAIL;
            IF public.scd_xact_origin(public.scd_unique_race(nsp, rel, con, det, snap)) <> 0 THEN
                RAISE EXCEPTION USING ERRCODE = 'unique_violation', MESSAGE = msg, DETAIL = det,
                    HINT = 'scd_wt: retry the transaction, a concurrent apply inserted this key',
                    SCHEMA = nsp, TABLE = rel, CONSTRAINT = con;
            END IF;
            RAISE;
        END;
        PERFORM set_config('scd.cmd', '', true);
        IF jsonb_typeof(res) IS DISTINCT FROM 'object' THEN
            RAISE EXCEPTION 'scd_wt: kind % returned %, expected {"touched": [...], "output": ...}', scd_wt.kind, coalesce(jsonb_typeof(res), 'null')
                USING ERRCODE = 'invalid_parameter_value';
        END IF;
        output := res -> 'output';
    END IF;
    IF NOT shipped THEN
        RETURN;
    END IF;
    FOR k IN SELECT e.value FROM jsonb_array_elements(coalesce(res -> 'touched', '[]')) AS e LOOP
        touched := touched || jsonb_build_array(jsonb_build_object(
            'tbl', k ->> 'tbl', 'pk', k -> 'pk', 'hash', public.scd_row_hash(k ->> 'tbl', k -> 'pk')));
        INSERT INTO public.cmd_pending (tbl, pk, node, id)
        VALUES (k ->> 'tbl', k -> 'pk', public.scd_node(), id)
        ON CONFLICT DO NOTHING;
    END LOOP;
    INSERT INTO public.cmd_outbox (node, id, kind, args, touched)
    VALUES (public.scd_node(), id, scd_wt.kind, args, touched);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_dispatch()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET session_replication_role = origin
SET lock_timeout = '5s'
SET search_path = public, pg_catalog
SET timezone TO 'UTC'
AS $f$
DECLARE
    out jsonb;
    k jsonb;
    h text;
    st text;
    msg text;
    snap pg_snapshot;
    nsp text;
    rel text;
    con text;
    det text;
BEGIN
    PERFORM 1 FROM public.cmd_done d WHERE d.node = NEW.node AND d.id = NEW.id;
    IF FOUND THEN
        RETURN NULL;
    END IF;
    INSERT INTO public.cmd_done (node, id, status) VALUES (NEW.node, NEW.id, 'ok');
    PERFORM set_config('scd.cmd', NEW.id::text, true);
    snap := pg_current_snapshot();
    BEGIN
        PERFORM 1 FROM public.scd_kinds() x WHERE x.kind = NEW.kind;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'scd dispatch: unknown kind %', NEW.kind USING ERRCODE = 'invalid_parameter_value';
        END IF;
        EXECUTE format('SELECT public.%I($1)', public.scd_kind_function(NEW.kind)) INTO out USING NEW.args;
    EXCEPTION WHEN OTHERS THEN
        GET STACKED DIAGNOSTICS st = RETURNED_SQLSTATE, msg = MESSAGE_TEXT,
            nsp = SCHEMA_NAME, rel = TABLE_NAME, con = CONSTRAINT_NAME, det = PG_EXCEPTION_DETAIL;
        IF public.scd_transient(st) THEN
            RAISE;
        END IF;
        IF st = '23505' AND public.scd_unique_race(nsp, rel, con, det, snap) IS NOT NULL THEN
            RAISE;
        END IF;
        INSERT INTO public.cmd_dead (node, id, kind, args, sqlstate, message)
        VALUES (NEW.node, NEW.id, NEW.kind, NEW.args, st, msg)
        ON CONFLICT DO NOTHING;
        UPDATE public.cmd_done d SET status = 'dead' WHERE d.node = NEW.node AND d.id = NEW.id;
    END;
    PERFORM set_config('scd.cmd', '', true);
    BEGIN
        FOR k IN SELECT e.value FROM jsonb_array_elements(coalesce(NEW.touched, '[]')) AS e LOOP
            CONTINUE WHEN public.scd_table_class(k ->> 'tbl') IS DISTINCT FROM 'C';
            h := public.scd_row_hash(k ->> 'tbl', k -> 'pk');
            IF h IS NULL AND k ->> 'hash' IS NOT NULL THEN
                INSERT INTO public.cmd_purge (node, id, tbl, pk)
                VALUES (NEW.node, NEW.id, k ->> 'tbl', k -> 'pk')
                ON CONFLICT DO NOTHING;
            ELSIF h IS NOT NULL AND h IS DISTINCT FROM k ->> 'hash' THEN
                INSERT INTO public.cmd_reassert (node, id, tbl, pk, "row")
                VALUES (NEW.node, NEW.id, k ->> 'tbl', k -> 'pk', public.scd_row_json(k ->> 'tbl', k -> 'pk'))
                ON CONFLICT DO NOTHING;
            END IF;
        END LOOP;
    EXCEPTION WHEN OTHERS THEN
        GET STACKED DIAGNOSTICS st = RETURNED_SQLSTATE, msg = MESSAGE_TEXT;
        IF public.scd_transient(st) THEN
            RAISE;
        END IF;
        INSERT INTO public.cmd_dead (node, id, kind, args, sqlstate, message)
        VALUES (NEW.node, NEW.id, NEW.kind, NEW.args, st, 'touched: ' || msg)
        ON CONFLICT DO NOTHING;
        UPDATE public.cmd_done d SET status = 'dead' WHERE d.node = NEW.node AND d.id = NEW.id;
    END;
    RETURN NULL;
END
$f$;

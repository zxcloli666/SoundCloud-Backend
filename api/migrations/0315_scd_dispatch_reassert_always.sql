SET LOCAL lock_timeout = '5s';

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
            ELSIF h IS NOT NULL AND (public.scd_reassert_always(k ->> 'tbl') OR h IS DISTINCT FROM k ->> 'hash') THEN
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

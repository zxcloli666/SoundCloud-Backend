SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_transient(sqlstate text)
RETURNS boolean
LANGUAGE sql
IMMUTABLE
AS $f$
SELECT left(sqlstate, 2) IN ('08', '40', '53', '57', '58', 'XX') OR sqlstate IN ('55P03', '55006')
$f$;

CREATE OR REPLACE FUNCTION public.scd_kind_function(kind text)
RETURNS text
LANGUAGE sql
IMMUTABLE
AS $f$
SELECT 'scd_cmd_' || replace(kind, '.', '_')
$f$;

CREATE OR REPLACE FUNCTION public.scd_wt(kind text, args jsonb)
RETURNS uuid
LANGUAGE plpgsql
SET search_path = public, pg_catalog
SET timezone TO 'UTC'
AS $f$
DECLARE
    cid uuid := gen_random_uuid();
    optimistic boolean;
    shipped boolean := public.scd_mode() = 'split' AND public.scd_role() = 'api';
    out jsonb;
    k jsonb;
    touched jsonb := '[]';
BEGIN
    SELECT x.optimistic INTO optimistic FROM public.scd_kinds() x WHERE x.kind = scd_wt.kind;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'scd_wt: unknown kind %', scd_wt.kind USING ERRCODE = 'invalid_parameter_value';
    END IF;
    IF optimistic OR NOT shipped THEN
        PERFORM set_config('scd.cmd', cid::text, true);
        EXECUTE format('SELECT public.%I($1)', public.scd_kind_function(scd_wt.kind)) INTO out USING args;
        PERFORM set_config('scd.cmd', '', true);
    END IF;
    IF NOT shipped THEN
        RETURN cid;
    END IF;
    FOR k IN SELECT e.value FROM jsonb_array_elements(coalesce(out, '[]')) AS e LOOP
        touched := touched || jsonb_build_array(jsonb_build_object(
            'tbl', k ->> 'tbl', 'pk', k -> 'pk', 'hash', public.scd_row_hash(k ->> 'tbl', k -> 'pk')));
        INSERT INTO public.cmd_pending (tbl, pk, node, id)
        VALUES (k ->> 'tbl', k -> 'pk', public.scd_node(), cid)
        ON CONFLICT DO NOTHING;
    END LOOP;
    INSERT INTO public.cmd_outbox (node, id, kind, args, touched)
    VALUES (public.scd_node(), cid, scd_wt.kind, args, touched);
    RETURN cid;
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
BEGIN
    PERFORM 1 FROM public.cmd_done d WHERE d.node = NEW.node AND d.id = NEW.id;
    IF FOUND THEN
        RETURN NULL;
    END IF;
    INSERT INTO public.cmd_done (node, id, status) VALUES (NEW.node, NEW.id, 'ok');
    PERFORM set_config('scd.cmd', NEW.id::text, true);
    BEGIN
        PERFORM 1 FROM public.scd_kinds() x WHERE x.kind = NEW.kind;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'scd dispatch: unknown kind %', NEW.kind USING ERRCODE = 'invalid_parameter_value';
        END IF;
        EXECUTE format('SELECT public.%I($1)', public.scd_kind_function(NEW.kind)) INTO out USING NEW.args;
    EXCEPTION WHEN OTHERS THEN
        GET STACKED DIAGNOSTICS st = RETURNED_SQLSTATE, msg = MESSAGE_TEXT;
        IF public.scd_transient(st) THEN
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

CREATE OR REPLACE TRIGGER zz_cmd_dispatch BEFORE INSERT ON public.cmd_outbox FOR EACH ROW
    WHEN (public.scd_mode() = 'split' AND public.scd_role() = 'core')
    EXECUTE FUNCTION public.scd_dispatch();
ALTER TABLE public.cmd_outbox ENABLE REPLICA TRIGGER zz_cmd_dispatch;

CREATE OR REPLACE FUNCTION public.scd_apply_row(tbl text, r jsonb)
RETURNS void
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    cols text;
BEGIN
    IF r IS NULL THEN
        RETURN;
    END IF;
    SELECT string_agg(quote_ident(a.attname), ', ' ORDER BY a.attnum) INTO cols
    FROM pg_attribute a
    WHERE a.attrelid = ('public.' || quote_ident(tbl))::regclass
      AND a.attnum > 0 AND NOT a.attisdropped AND a.attgenerated = '' AND r ? a.attname;
    IF cols IS NULL THEN
        RETURN;
    END IF;
    EXECUTE format('INSERT INTO public.%I (%s) SELECT %s FROM jsonb_populate_record(NULL::public.%I, $1) ON CONFLICT DO NOTHING',
        tbl, cols, cols, tbl) USING r;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_on_cmd_done()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
BEGIN
    IF NEW.node = public.scd_node() THEN
        DELETE FROM public.cmd_pending p WHERE p.node = NEW.node AND p.id = NEW.id;
        UPDATE public.cmd_outbox o SET done_at = now() WHERE o.node = NEW.node AND o.id = NEW.id AND o.done_at IS NULL;
    END IF;
    RETURN NULL;
END
$f$;

CREATE OR REPLACE TRIGGER zz_cmd_done BEFORE INSERT ON public.cmd_done FOR EACH ROW
    WHEN (public.scd_mode() = 'split' AND public.scd_role() = 'api')
    EXECUTE FUNCTION public.scd_on_cmd_done();
ALTER TABLE public.cmd_done ENABLE REPLICA TRIGGER zz_cmd_done;

CREATE OR REPLACE FUNCTION public.scd_on_cmd_reassert()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
BEGIN
    IF NEW.node = public.scd_node() AND public.scd_table_class(NEW.tbl) = 'C' THEN
        PERFORM public.scd_apply_row(NEW.tbl, NEW."row");
    END IF;
    RETURN NULL;
END
$f$;

CREATE OR REPLACE TRIGGER zz_cmd_reassert BEFORE INSERT ON public.cmd_reassert FOR EACH ROW
    WHEN (public.scd_mode() = 'split' AND public.scd_role() = 'api')
    EXECUTE FUNCTION public.scd_on_cmd_reassert();
ALTER TABLE public.cmd_reassert ENABLE REPLICA TRIGGER zz_cmd_reassert;

CREATE OR REPLACE FUNCTION public.scd_on_cmd_purge()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
BEGIN
    IF NEW.node = public.scd_node() AND public.scd_table_class(NEW.tbl) = 'C'
        AND NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = NEW.tbl AND p.pk = NEW.pk) THEN
        PERFORM public.scd_purge(NEW.tbl, NEW.pk);
    END IF;
    RETURN NULL;
END
$f$;

CREATE OR REPLACE TRIGGER zz_cmd_purge BEFORE INSERT ON public.cmd_purge FOR EACH ROW
    WHEN (public.scd_mode() = 'split' AND public.scd_role() = 'api')
    EXECUTE FUNCTION public.scd_on_cmd_purge();
ALTER TABLE public.cmd_purge ENABLE REPLICA TRIGGER zz_cmd_purge;

CREATE OR REPLACE FUNCTION public.scd_on_repl_repair()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    cls text := public.scd_table_class(NEW.tbl);
BEGIN
    IF NEW.src_node <> public.scd_node()
        AND ((public.scd_role() = 'core' AND cls IN ('M', 'A', 'K')) OR (public.scd_role() = 'api' AND cls IN ('C', 'M', 'A'))) THEN
        PERFORM public.scd_apply_row(NEW.tbl, NEW."row");
    END IF;
    RETURN NULL;
END
$f$;

CREATE OR REPLACE TRIGGER zz_repl_repair BEFORE INSERT ON public.repl_repair FOR EACH ROW
    WHEN (public.scd_mode() = 'split')
    EXECUTE FUNCTION public.scd_on_repl_repair();
ALTER TABLE public.repl_repair ENABLE REPLICA TRIGGER zz_repl_repair;

CREATE OR REPLACE FUNCTION public.scd_drain_resend(batch integer DEFAULT 500)
RETURNS integer
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    r record;
    body jsonb;
    rid uuid;
    n integer := 0;
BEGIN
    FOR r IN
        SELECT x.tbl, x.pk FROM public.repl_resend x ORDER BY x.tbl, x.pk LIMIT batch FOR UPDATE SKIP LOCKED
    LOOP
        body := public.scd_row_json(r.tbl, r.pk);
        IF body IS NOT NULL THEN
            INSERT INTO public.repl_repair (id, src_node, tbl, pk, "row")
            VALUES (gen_random_uuid(), public.scd_node(), r.tbl, r.pk, body)
            RETURNING id INTO rid;
            DELETE FROM public.repl_repair x WHERE x.id = rid;
        END IF;
        DELETE FROM public.repl_resend x WHERE x.tbl = r.tbl AND x.pk = r.pk;
        n := n + 1;
    END LOOP;
    RETURN n;
END
$f$;

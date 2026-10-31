SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_node()
RETURNS text
LANGUAGE sql
STABLE
AS $f$
SELECT coalesce(nullif(current_setting('scd.node', true), ''), 'core')
$f$;

CREATE OR REPLACE FUNCTION public.scd_role()
RETURNS text
LANGUAGE sql
STABLE
AS $f$
SELECT coalesce(nullif(current_setting('scd.role', true), ''), 'core')
$f$;

CREATE OR REPLACE FUNCTION public.scd_mode()
RETURNS text
LANGUAGE sql
STABLE
AS $f$
SELECT coalesce(nullif(current_setting('scd.mode', true), ''), 'legacy')
$f$;

CREATE OR REPLACE FUNCTION public.scd_node_ord()
RETURNS bigint
LANGUAGE sql
STABLE
AS $f$
SELECT CASE public.scd_node() WHEN 'core' THEN 1 WHEN 'main' THEN 2 WHEN 'star' THEN 3 END::bigint
$f$;

CREATE OR REPLACE FUNCTION public.scd_hlc(prev bigint)
RETURNS bigint
LANGUAGE sql
VOLATILE
AS $f$
SELECT (greatest((extract(epoch FROM clock_timestamp()) * 1000000)::bigint, (coalesce(prev, 0) >> 4) + 1) << 4)
    + public.scd_node_ord()
$f$;

CREATE OR REPLACE FUNCTION public.scd_hlc_offset_ms(v bigint)
RETURNS numeric
LANGUAGE sql
VOLATILE
AS $f$
SELECT round(((v >> 4) - (extract(epoch FROM clock_timestamp()) * 1000000)::bigint) / 1000.0, 1)
$f$;

CREATE OR REPLACE FUNCTION public.scd_clock()
RETURNS bigint
LANGUAGE sql
VOLATILE
AS $f$
SELECT public.scd_hlc(0)
$f$;

CREATE OR REPLACE FUNCTION public.scd_uuid_v3(namespace uuid, name text)
RETURNS uuid
LANGUAGE sql
IMMUTABLE
STRICT
AS $f$
SELECT overlay(overlay(h PLACING '3' FROM 13) PLACING to_hex((('x' || substr(h, 17, 1))::bit(4)::int & 3) | 8) FROM 17)::uuid
FROM (SELECT md5(decode(replace(namespace::text, '-', ''), 'hex') || convert_to(name, 'UTF8')) AS h) AS x
$f$;

CREATE OR REPLACE FUNCTION public.scd_uuid(ns text, VARIADIC key text[])
RETURNS uuid
LANGUAGE sql
IMMUTABLE
AS $f$
SELECT CASE
    WHEN ns IS NULL OR key IS NULL OR cardinality(key) = 0 OR array_position(key, NULL) IS NOT NULL THEN NULL
    ELSE public.scd_uuid_v3('e5f6011f-72fa-46e1-ad97-55586f6f1f83', ns || ':' || array_to_string(key, E'\x1f'))
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_derived()
RETURNS boolean
LANGUAGE sql
STABLE
AS $f$
SELECT CASE
    WHEN public.scd_mode() = 'split' THEN public.scd_role() = 'core'
    ELSE current_setting('session_replication_role') <> 'replica'
END
$f$;

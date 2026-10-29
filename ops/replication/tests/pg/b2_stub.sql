CREATE OR REPLACE FUNCTION public.scd_role() RETURNS text LANGUAGE sql STABLE
AS $f$ SELECT coalesce(nullif(current_setting('scd.role', true), ''), 'core') $f$;

CREATE OR REPLACE FUNCTION public.scd_mode() RETURNS text LANGUAGE sql STABLE
AS $f$ SELECT coalesce(nullif(current_setting('scd.mode', true), ''), 'legacy') $f$;

CREATE OR REPLACE FUNCTION public.scd_node() RETURNS text LANGUAGE sql STABLE
AS $f$ SELECT coalesce(nullif(current_setting('scd.node', true), ''), 'core') $f$;

CREATE OR REPLACE FUNCTION public.scd_hlc(prev bigint) RETURNS bigint LANGUAGE sql VOLATILE
AS $f$
SELECT (greatest((extract(epoch FROM clock_timestamp()) * 1000000)::bigint, (coalesce(prev, 0) >> 4) + 1) << 4)
    + CASE public.scd_node() WHEN 'core' THEN 1 WHEN 'main' THEN 2 WHEN 'star' THEN 3 END
$f$;

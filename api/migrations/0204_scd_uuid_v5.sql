SET LOCAL lock_timeout = '5s';

CREATE EXTENSION IF NOT EXISTS "uuid-ossp" WITH SCHEMA public;

CREATE OR REPLACE FUNCTION public.scd_uuid(ns text, VARIADIC key text[])
RETURNS uuid
LANGUAGE sql
IMMUTABLE
AS $f$
SELECT CASE
    WHEN ns IS NULL OR key IS NULL OR cardinality(key) = 0 OR array_position(key, NULL) IS NOT NULL THEN NULL
    ELSE public.uuid_generate_v5('e5f6011f-72fa-46e1-ad97-55586f6f1f83', ns || ':' || array_to_string(key, E'\x1f'))
END
$f$;

DROP FUNCTION IF EXISTS public.scd_uuid_v3(uuid, text);

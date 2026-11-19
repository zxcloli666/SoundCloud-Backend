SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_cmd_cold_refresh_clear(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
AS $f$
DECLARE
    touched jsonb := '[]';
    part jsonb;
    cleared bigint := 0;
    n bigint;
BEGIN
    IF args ->> 'mode' = 'audience' THEN
        WITH gone AS (
            DELETE FROM public.catalog_audience WHERE subject_urn = args ->> 'subject_urn'
            RETURNING subject_urn, relation, user_urn
        )
        SELECT count(*), coalesce(jsonb_agg(jsonb_build_object('tbl', 'catalog_audience', 'pk',
                   jsonb_build_object('subject_urn', subject_urn, 'relation', relation, 'user_urn', user_urn))), '[]')
        INTO n, part FROM gone;
        cleared := cleared + n;
        touched := touched || part;
        WITH gone AS (
            DELETE FROM public.catalog_audience_ids
            WHERE subject_id = substring(args ->> 'subject_id' FROM '^[0-9]{1,18}$')::bigint
              AND relation IN (
                  SELECT CASE name
                             WHEN 'track-favoriters' THEN 1
                             WHEN 'track-reposters' THEN 2
                             WHEN 'playlist-reposters' THEN 3
                         END
                  FROM jsonb_array_elements_text(args -> 'relations') AS name
              )
            RETURNING subject_id, relation, user_id
        )
        SELECT coalesce(jsonb_agg(jsonb_build_object('tbl', 'catalog_audience_ids', 'pk',
                   jsonb_build_object('subject_id', subject_id, 'relation', relation, 'user_id', user_id))), '[]')
        INTO part FROM gone;
        touched := touched || part;
        WITH gone AS (
            DELETE FROM public.catalog_collection_sync
            WHERE subject_id = args ->> 'subject_id' AND scope = 'public'
              AND collection = ANY (ARRAY(SELECT jsonb_array_elements_text(args -> 'relations')))
            RETURNING subject_id, collection, scope
        )
        SELECT count(*), coalesce(jsonb_agg(jsonb_build_object('tbl', 'catalog_collection_sync', 'pk',
                   jsonb_build_object('subject_id', subject_id, 'collection', collection, 'scope', scope))), '[]')
        INTO n, part FROM gone;
    ELSE
        WITH gone AS (
            DELETE FROM public.track_comments WHERE sc_track_id = args ->> 'sc_track_id'
            RETURNING id
        )
        SELECT count(*), coalesce(jsonb_agg(jsonb_build_object('tbl', 'track_comments', 'pk', jsonb_build_object('id', id))), '[]')
        INTO n, part FROM gone;
        cleared := cleared + n;
        touched := touched || part;
        WITH gone AS (
            DELETE FROM public.catalog_collection_sync
            WHERE subject_id = args ->> 'sc_track_id' AND scope = 'public' AND collection = 'track-comments'
            RETURNING subject_id, collection, scope
        )
        SELECT count(*), coalesce(jsonb_agg(jsonb_build_object('tbl', 'catalog_collection_sync', 'pk',
                   jsonb_build_object('subject_id', subject_id, 'collection', collection, 'scope', scope))), '[]')
        INTO n, part FROM gone;
    END IF;
    cleared := cleared + n;
    touched := touched || part;
    RETURN jsonb_build_object('touched', touched, 'output', cleared);
END
$f$;

DO $$
DECLARE
    broken text;
BEGIN
    SELECT string_agg(name, ', ' ORDER BY name)
      INTO broken
      FROM unnest(ARRAY[
          'background_job_failures_dedup_failed_idx',
          'tracks_search_doc_gin',
          'playlists_search_doc_gin',
          'users_search_doc_gin',
          'artists_search_doc_gin',
          'albums_search_doc_gin',
          'tracks_permalink_url_lower_idx',
          'playlists_permalink_url_lower_idx',
          'users_permalink_url_lower_idx'
      ]) AS name
     WHERE NOT EXISTS (
         SELECT 1
           FROM pg_index i
          WHERE i.indexrelid = to_regclass(name)
            AND i.indisvalid
            AND i.indisready
     );

    IF broken IS NOT NULL THEN
        RAISE EXCEPTION 'missing or invalid concurrent indexes: %', broken
            USING HINT = 'run DROP INDEX CONCURRENTLY IF EXISTS <name> for each, then rerun migrate core';
    END IF;
END
$$;

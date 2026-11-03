SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_cmd_observe_tracks(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    stamp timestamptz := (args ->> 'observed_at')::timestamptz;
    seen bigint := (args ->> 'observation')::bigint;
    r record;
    row_id uuid;
    fresh boolean;
    applied boolean;
    written boolean;
    output jsonb := '[]';
    touched jsonb := '[]';
BEGIN
    IF args ->> 'mode' = 'bump' THEN
        WITH bumped AS (
            UPDATE public.tracks AS t
            SET index_priority = LEAST(t.index_priority, b.index_priority),
                storage_priority = LEAST(t.storage_priority, b.storage_priority),
                pipeline_held = false
            FROM jsonb_to_recordset(args -> 'rows') AS b(sc_track_id text, index_priority smallint, storage_priority smallint)
            WHERE t.sc_track_id = b.sc_track_id
              AND (t.index_priority > b.index_priority OR t.storage_priority > b.storage_priority OR t.pipeline_held)
            RETURNING t.id
        )
        SELECT coalesce(jsonb_agg(jsonb_build_object('tbl', 'tracks', 'pk', jsonb_build_object('id', bumped.id))), '[]')
        INTO touched
        FROM bumped;
        RETURN jsonb_build_object('touched', touched, 'output', output);
    END IF;

    IF args ->> 'mode' = 'absent' THEN
        WITH inserted AS (
            INSERT INTO public.tracks (
                id, sc_track_id, urn, title, title_normalized, description, genre, tags,
                duration_ms, artwork_url, permalink_url, waveform_url, language, isrc,
                metadata_artist, sharing, sc_created_at, sc_last_modified, release_year, release_date,
                uploader_sc_user_id, uploader_urn, uploader_username, uploader_avatar_url,
                play_count_sc, likes_count_sc, reposts_count_sc, comments_count_sc,
                needs_duration_resolve, index_priority, storage_priority, is_cover,
                sc_synced_at, sc_observation, sc_metadata, pipeline_held,
                created_at, updated_at, enrich_next_run_at
            )
            SELECT public.scd_uuid('tracks', row.urn), row.sc_track_id, row.urn, row.title, row.title_normalized,
                   row.description, row.genre, row.tags, row.duration_ms, row.artwork_url, row.permalink_url,
                   row.waveform_url, row.language, row.isrc, row.metadata_artist, row.sharing, row.sc_created_at,
                   row.sc_last_modified, row.release_year, row.release_date, row.uploader_sc_user_id,
                   row.uploader_urn, row.uploader_username, row.uploader_avatar_url, row.play_count_sc,
                   row.likes_count_sc, row.reposts_count_sc, row.comments_count_sc,
                   row.needs_duration_resolve, row.index_priority, row.storage_priority, row.is_cover,
                   stamp, seen, row.sc_metadata, true, stamp, stamp, stamp
            FROM jsonb_to_recordset(args -> 'rows') AS row(
                sc_track_id text, urn text, title text, title_normalized text, description text,
                genre text, tags text[], duration_ms integer, artwork_url text, permalink_url text,
                waveform_url text, language text, isrc text, metadata_artist text, sharing text,
                sc_created_at timestamptz, sc_last_modified timestamptz, release_year smallint,
                release_date date, uploader_sc_user_id text, uploader_urn text, uploader_username text,
                uploader_avatar_url text, play_count_sc bigint, likes_count_sc bigint,
                reposts_count_sc bigint, comments_count_sc bigint, needs_duration_resolve boolean,
                is_cover boolean, sc_metadata jsonb, index_priority smallint, storage_priority smallint
            )
            ON CONFLICT DO NOTHING
            RETURNING id, sc_track_id
        )
        SELECT coalesce(jsonb_agg(jsonb_build_object('sc_track_id', inserted.sc_track_id, 'id', inserted.id, 'was_new', true, 'applied', true)), '[]'),
               coalesce(jsonb_agg(jsonb_build_object('tbl', 'tracks', 'pk', jsonb_build_object('id', inserted.id))), '[]')
        INTO output, touched
        FROM inserted;
        RETURN jsonb_build_object('touched', touched, 'output', output);
    END IF;

    FOR r IN
        SELECT *
        FROM jsonb_to_recordset(args -> 'rows') AS row(
            sc_track_id text, urn text, title text, title_normalized text, description text,
            genre text, tags text[], duration_ms integer, artwork_url text, permalink_url text,
            waveform_url text, language text, isrc text, metadata_artist text, sharing text,
            sc_created_at timestamptz, sc_last_modified timestamptz, release_year smallint,
            release_date date, uploader_sc_user_id text, uploader_urn text, uploader_username text,
            uploader_avatar_url text, play_count_sc bigint, likes_count_sc bigint,
            reposts_count_sc bigint, comments_count_sc bigint, needs_duration_resolve boolean,
            is_cover boolean, sc_metadata jsonb, index_priority smallint, storage_priority smallint
        )
    LOOP
        INSERT INTO public.tracks AS tracks (
            id, sc_track_id, urn, title, title_normalized, description, genre, tags,
            duration_ms, artwork_url, permalink_url, waveform_url, language, isrc,
            metadata_artist, sharing, sc_created_at, sc_last_modified, release_year, release_date,
            uploader_sc_user_id, uploader_urn, uploader_username, uploader_avatar_url,
            play_count_sc, likes_count_sc, reposts_count_sc, comments_count_sc,
            needs_duration_resolve, index_priority, storage_priority, is_cover, sc_synced_at, sc_observation, sc_metadata,
            created_at, updated_at, enrich_next_run_at
        ) VALUES (
            public.scd_uuid('tracks', r.urn), r.sc_track_id, r.urn, r.title, r.title_normalized, r.description,
            r.genre, r.tags, r.duration_ms, r.artwork_url, r.permalink_url, r.waveform_url, r.language, r.isrc,
            r.metadata_artist, r.sharing, r.sc_created_at, r.sc_last_modified, r.release_year, r.release_date,
            r.uploader_sc_user_id, r.uploader_urn, r.uploader_username, r.uploader_avatar_url,
            r.play_count_sc, r.likes_count_sc, r.reposts_count_sc, r.comments_count_sc,
            r.needs_duration_resolve, r.index_priority, r.storage_priority, r.is_cover, stamp, seen, r.sc_metadata,
            stamp, stamp, stamp
        )
        ON CONFLICT (sc_track_id) DO UPDATE SET
            urn = EXCLUDED.urn,
            title = EXCLUDED.title,
            title_normalized = EXCLUDED.title_normalized,
            description = EXCLUDED.description,
            genre = EXCLUDED.genre,
            tags = EXCLUDED.tags,
            duration_ms = CASE
                WHEN EXCLUDED.duration_ms > 0 THEN EXCLUDED.duration_ms
                ELSE tracks.duration_ms
            END,
            storage_state = CASE
                WHEN tracks.storage_state = 'failed'
                     AND EXCLUDED.duration_ms > 0
                     AND EXCLUDED.duration_ms IS DISTINCT FROM tracks.duration_ms
                    THEN 'pending'
                ELSE tracks.storage_state
            END,
            storage_attempts = CASE
                WHEN tracks.storage_state = 'failed'
                     AND EXCLUDED.duration_ms > 0
                     AND EXCLUDED.duration_ms IS DISTINCT FROM tracks.duration_ms
                    THEN 0
                ELSE tracks.storage_attempts
            END,
            artwork_url = EXCLUDED.artwork_url,
            permalink_url = EXCLUDED.permalink_url,
            waveform_url = EXCLUDED.waveform_url,
            language = COALESCE(EXCLUDED.language, tracks.language),
            isrc = COALESCE(EXCLUDED.isrc, tracks.isrc),
            metadata_artist = COALESCE(EXCLUDED.metadata_artist, tracks.metadata_artist),
            sharing = EXCLUDED.sharing,
            sc_metadata = EXCLUDED.sc_metadata,
            sc_created_at = COALESCE(EXCLUDED.sc_created_at, tracks.sc_created_at),
            sc_last_modified = COALESCE(EXCLUDED.sc_last_modified, tracks.sc_last_modified),
            release_year = COALESCE(EXCLUDED.release_year, tracks.release_year),
            release_date = COALESCE(EXCLUDED.release_date, tracks.release_date),
            uploader_sc_user_id = COALESCE(EXCLUDED.uploader_sc_user_id, tracks.uploader_sc_user_id),
            uploader_urn = COALESCE(EXCLUDED.uploader_urn, tracks.uploader_urn),
            uploader_username = COALESCE(EXCLUDED.uploader_username, tracks.uploader_username),
            uploader_avatar_url = COALESCE(EXCLUDED.uploader_avatar_url, tracks.uploader_avatar_url),
            play_count_sc = COALESCE(EXCLUDED.play_count_sc, tracks.play_count_sc),
            likes_count_sc = COALESCE(EXCLUDED.likes_count_sc, tracks.likes_count_sc),
            reposts_count_sc = COALESCE(EXCLUDED.reposts_count_sc, tracks.reposts_count_sc),
            comments_count_sc = COALESCE(EXCLUDED.comments_count_sc, tracks.comments_count_sc),
            needs_duration_resolve = EXCLUDED.needs_duration_resolve,
            duration_resolve_attempts = CASE
                WHEN tracks.needs_duration_resolve IS DISTINCT FROM EXCLUDED.needs_duration_resolve
                    THEN 0
                ELSE tracks.duration_resolve_attempts
            END,
            duration_resolve_retry_at = CASE
                WHEN tracks.needs_duration_resolve IS DISTINCT FROM EXCLUDED.needs_duration_resolve
                    THEN NULL
                ELSE tracks.duration_resolve_retry_at
            END,
            index_priority = LEAST(tracks.index_priority, EXCLUDED.index_priority),
            storage_priority = LEAST(tracks.storage_priority, EXCLUDED.storage_priority),
            is_cover = tracks.is_cover OR EXCLUDED.is_cover,
            pipeline_held = false,
            sc_synced_at = stamp,
            sc_observation = GREATEST(tracks.sc_observation, EXCLUDED.sc_observation),
            sc_desired = '{}',
            sc_write_confirmed = false,
            updated_at = stamp
        WHERE tracks.deleted_at IS NULL
            AND (
             EXCLUDED.sc_last_modified IS NULL
             OR tracks.sc_synced_at < stamp - interval '7 days'
             OR tracks.sc_desired <> '{}'::jsonb
             OR tracks.sc_last_modified IS DISTINCT FROM COALESCE(EXCLUDED.sc_last_modified, tracks.sc_last_modified)
             OR tracks.title IS DISTINCT FROM EXCLUDED.title
             OR tracks.sharing IS DISTINCT FROM EXCLUDED.sharing
             OR tracks.artwork_url IS DISTINCT FROM EXCLUDED.artwork_url
             OR tracks.permalink_url IS DISTINCT FROM EXCLUDED.permalink_url
             OR tracks.needs_duration_resolve IS DISTINCT FROM EXCLUDED.needs_duration_resolve
             OR (EXCLUDED.duration_ms > 0 AND tracks.duration_ms IS DISTINCT FROM EXCLUDED.duration_ms)
             OR (EXCLUDED.is_cover AND NOT tracks.is_cover)
             OR (tracks.uploader_sc_user_id IS NULL AND EXCLUDED.uploader_sc_user_id IS NOT NULL)
             OR tracks.pipeline_held
         )
            AND public.catalog_observation_is_current(
            tracks.sc_observation, tracks.sc_mutation_observation, EXCLUDED.sc_observation,
            tracks.sc_last_modified, COALESCE(EXCLUDED.sc_last_modified, tracks.sc_last_modified),
            tracks.sc_desired, tracks.sc_write_confirmed, to_jsonb(EXCLUDED)
         )
        RETURNING tracks.id, (tracks.xmax = 0) INTO row_id, fresh;
        applied := FOUND;
        written := applied;
        IF NOT applied THEN
            fresh := false;
            UPDATE public.tracks AS t
            SET index_priority = LEAST(t.index_priority, r.index_priority),
                storage_priority = LEAST(t.storage_priority, r.storage_priority),
                pipeline_held = false
            WHERE t.sc_track_id = r.sc_track_id
              AND (t.index_priority > r.index_priority OR t.storage_priority > r.storage_priority OR t.pipeline_held)
            RETURNING t.id INTO row_id;
            written := FOUND;
            IF NOT written THEN
                SELECT t.id INTO row_id FROM public.tracks AS t WHERE t.sc_track_id = r.sc_track_id;
            END IF;
        END IF;
        output := output || jsonb_build_array(jsonb_build_object(
            'sc_track_id', r.sc_track_id, 'id', row_id, 'was_new', fresh, 'applied', applied));
        IF written THEN
            touched := touched || jsonb_build_array(jsonb_build_object('tbl', 'tracks', 'pk', jsonb_build_object('id', row_id)));
        END IF;
    END LOOP;
    RETURN jsonb_build_object('touched', touched, 'output', output);
END
$f$;

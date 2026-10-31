SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_cmd_observe_playlists(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    stamp timestamptz := (args ->> 'observed_at')::timestamptz;
    seen bigint := (args ->> 'observation')::bigint;
    r record;
    fresh boolean;
    claimed boolean;
    output jsonb := '[]';
    touched jsonb := '[]';
BEGIN
    IF args ->> 'mode' = 'absent' THEN
        WITH inserted AS (
            INSERT INTO public.playlists (
                urn, sc_playlist_id, title, title_normalized, description, genre, tags,
                artwork_url, permalink_url, owner_sc_user_id, owner_urn, owner_username,
                track_count, duration_ms, playlist_type, kind, sharing,
                release_year, release_date, label_name, likes_count_sc, reposts_count_sc,
                sc_created_at, sc_last_modified, sc_synced_at, sc_observation, sc_metadata,
                created_at, updated_at
            )
            SELECT row.urn, row.sc_playlist_id, row.title, row.title_normalized, row.description,
                   row.genre, row.tags, row.artwork_url, row.permalink_url, row.owner_sc_user_id,
                   row.owner_urn, row.owner_username, row.track_count, row.duration_ms, row.playlist_type,
                   row.kind, row.sharing, row.release_year, row.release_date, row.label_name,
                   row.likes_count_sc, row.reposts_count_sc, row.sc_created_at, row.sc_last_modified,
                   stamp, seen, row.sc_metadata, stamp, stamp
            FROM jsonb_to_recordset(args -> 'rows') AS row(
                urn text, sc_playlist_id text, title text, title_normalized text, description text,
                genre text, tags text[], artwork_url text, permalink_url text, owner_sc_user_id text,
                owner_urn text, owner_username text, track_count integer, duration_ms bigint,
                playlist_type text, kind text, sharing text, release_year smallint, release_date date,
                label_name text, likes_count_sc bigint, reposts_count_sc bigint,
                sc_created_at timestamptz, sc_last_modified timestamptz, sc_metadata jsonb
            )
            ORDER BY row.urn
            ON CONFLICT DO NOTHING
            RETURNING urn
        )
        SELECT coalesce(jsonb_agg(jsonb_build_object('urn', inserted.urn, 'was_new', true)), '[]'),
               coalesce(jsonb_agg(jsonb_build_object('tbl', 'playlists', 'pk', jsonb_build_object('urn', inserted.urn))), '[]')
        INTO output, touched
        FROM inserted;
        RETURN jsonb_build_object('touched', touched, 'output', output);
    END IF;

    FOR r IN
        SELECT *
        FROM jsonb_to_recordset(args -> 'rows') AS row(
            urn text, sc_playlist_id text, title text, title_normalized text, description text,
            genre text, tags text[], artwork_url text, permalink_url text, owner_sc_user_id text,
            owner_urn text, owner_username text, track_count integer, duration_ms bigint,
            playlist_type text, kind text, sharing text, release_year smallint, release_date date,
            label_name text, likes_count_sc bigint, reposts_count_sc bigint,
            sc_created_at timestamptz, sc_last_modified timestamptz, sc_metadata jsonb
        )
        ORDER BY row.urn
    LOOP
        claimed := NULL;
        INSERT INTO public.playlist_membership_state AS state (
            playlist_urn,
            projection_track_count,
            sync_status,
            next_reconcile_at,
            updated_at
        )
        SELECT playlist.urn,
               (
                   SELECT count(*)::integer
                   FROM public.playlist_track_projection AS projection
                   WHERE projection.playlist_urn = playlist.urn
               ),
               'unhydrated',
               stamp,
               stamp
        FROM public.playlists AS playlist
        WHERE playlist.urn = r.urn AND playlist.deleted_at IS NULL
        ON CONFLICT (playlist_urn) DO UPDATE
        SET updated_at = state.updated_at
        RETURNING state.xmax = 0 INTO claimed;
        IF claimed THEN
            touched := touched || jsonb_build_array(jsonb_build_object('tbl', 'playlist_membership_state', 'pk', jsonb_build_object('playlist_urn', r.urn)));
        END IF;
        INSERT INTO public.playlists AS playlists (
           urn, sc_playlist_id, title, title_normalized, description, genre, tags,
           artwork_url, permalink_url, owner_sc_user_id, owner_urn, owner_username,
           track_count, duration_ms, playlist_type, kind, sharing,
           release_year, release_date, label_name, likes_count_sc, reposts_count_sc,
           sc_created_at, sc_last_modified, sc_synced_at, sc_observation, sc_metadata,
           created_at, updated_at
        ) VALUES (
           r.urn, r.sc_playlist_id, r.title, r.title_normalized, r.description, r.genre, r.tags,
           r.artwork_url, r.permalink_url, r.owner_sc_user_id, r.owner_urn, r.owner_username,
           r.track_count, r.duration_ms, r.playlist_type, r.kind, r.sharing,
           r.release_year, r.release_date, r.label_name, r.likes_count_sc, r.reposts_count_sc,
           r.sc_created_at, r.sc_last_modified, stamp, seen, r.sc_metadata,
           stamp, stamp
        )
        ON CONFLICT (urn) DO UPDATE SET
           sc_playlist_id = EXCLUDED.sc_playlist_id,
           title = EXCLUDED.title,
           title_normalized = EXCLUDED.title_normalized,
           description = EXCLUDED.description,
           genre = EXCLUDED.genre,
           tags = EXCLUDED.tags,
           artwork_url = EXCLUDED.artwork_url,
           permalink_url = EXCLUDED.permalink_url,
           owner_sc_user_id = COALESCE(EXCLUDED.owner_sc_user_id, playlists.owner_sc_user_id),
           owner_urn = COALESCE(EXCLUDED.owner_urn, playlists.owner_urn),
           owner_username = COALESCE(EXCLUDED.owner_username, playlists.owner_username),
           track_count = EXCLUDED.track_count,
           duration_ms = COALESCE(EXCLUDED.duration_ms, playlists.duration_ms),
           playlist_type = COALESCE(EXCLUDED.playlist_type, playlists.playlist_type),
           kind = COALESCE(EXCLUDED.kind, playlists.kind),
           sharing = EXCLUDED.sharing,
           sc_metadata = EXCLUDED.sc_metadata,
           release_year = COALESCE(EXCLUDED.release_year, playlists.release_year),
           release_date = COALESCE(EXCLUDED.release_date, playlists.release_date),
           label_name = COALESCE(EXCLUDED.label_name, playlists.label_name),
           likes_count_sc = COALESCE(EXCLUDED.likes_count_sc, playlists.likes_count_sc),
           reposts_count_sc = COALESCE(EXCLUDED.reposts_count_sc, playlists.reposts_count_sc),
           sc_created_at = COALESCE(EXCLUDED.sc_created_at, playlists.sc_created_at),
           sc_last_modified = COALESCE(EXCLUDED.sc_last_modified, playlists.sc_last_modified),
           sc_synced_at = stamp,
           sc_observation = GREATEST(playlists.sc_observation, EXCLUDED.sc_observation),
           sc_desired = '{}',
           sc_write_confirmed = false,
           updated_at = stamp
        WHERE playlists.deleted_at IS NULL AND public.catalog_observation_is_current(
           playlists.sc_observation, playlists.sc_mutation_observation, EXCLUDED.sc_observation,
           playlists.sc_last_modified, COALESCE(EXCLUDED.sc_last_modified, playlists.sc_last_modified),
           playlists.sc_desired, playlists.sc_write_confirmed, to_jsonb(EXCLUDED)
        )
        RETURNING (playlists.xmax = 0) INTO fresh;
        IF FOUND THEN
            output := output || jsonb_build_array(jsonb_build_object('urn', r.urn, 'was_new', fresh));
            touched := touched || jsonb_build_array(jsonb_build_object('tbl', 'playlists', 'pk', jsonb_build_object('urn', r.urn)));
        END IF;
        INSERT INTO public.playlist_membership_state AS state (
            playlist_urn,
            projection_track_count,
            sync_status,
            next_reconcile_at,
            updated_at
        )
        SELECT playlist.urn,
               (
                   SELECT count(*)::integer
                   FROM public.playlist_track_projection AS projection
                   WHERE projection.playlist_urn = playlist.urn
               ),
               'unhydrated',
               stamp,
               stamp
        FROM public.playlists AS playlist
        WHERE playlist.urn = r.urn AND playlist.deleted_at IS NULL
        ON CONFLICT (playlist_urn) DO UPDATE
        SET next_reconcile_at = stamp,
            updated_at = stamp
        WHERE state.last_operation_sequence <= state.committed_operation_sequence
          AND (
              SELECT playlist.track_count
              FROM public.playlists AS playlist
              WHERE playlist.urn = r.urn AND playlist.deleted_at IS NULL
          ) <> COALESCE(
              (
                  SELECT baseline.declared_track_count
                  FROM public.playlist_remote_observations AS baseline
                  WHERE baseline.playlist_urn = state.playlist_urn
                    AND baseline.id = state.baseline_observation_id
              ),
              state.projection_track_count
          );
        IF FOUND AND claimed IS NOT TRUE THEN
            touched := touched || jsonb_build_array(jsonb_build_object('tbl', 'playlist_membership_state', 'pk', jsonb_build_object('playlist_urn', r.urn)));
        END IF;
    END LOOP;
    RETURN jsonb_build_object('touched', touched, 'output', output);
END
$f$;

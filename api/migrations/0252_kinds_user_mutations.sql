SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_cmd_track_edit(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    stamp timestamptz := (args ->> 'at')::timestamptz;
    seen bigint := (args ->> 'observation')::bigint;
    edit jsonb := args -> 'desired';
    edited uuid;
    edited_urn text;
BEGIN
    UPDATE public.tracks AS track
    SET title = CASE WHEN edit ? 'title' THEN edit ->> 'title' ELSE track.title END,
        title_normalized = CASE WHEN edit ? 'title_normalized' THEN edit ->> 'title_normalized' ELSE track.title_normalized END,
        description = CASE WHEN edit ? 'description' THEN edit ->> 'description' ELSE track.description END,
        genre = CASE WHEN edit ? 'genre' THEN edit ->> 'genre' ELSE track.genre END,
        tags = CASE WHEN edit ? 'tags' THEN ARRAY(SELECT jsonb_array_elements_text(edit -> 'tags')) ELSE track.tags END,
        sharing = CASE WHEN edit ? 'sharing' THEN edit ->> 'sharing' ELSE track.sharing END,
        isrc = CASE WHEN edit ? 'isrc' THEN edit ->> 'isrc' ELSE track.isrc END,
        metadata_artist = CASE WHEN edit ? 'metadata_artist' THEN edit ->> 'metadata_artist' ELSE track.metadata_artist END,
        release_date = CASE WHEN edit ? 'release_date' THEN (edit ->> 'release_date')::date ELSE track.release_date END,
        release_year = CASE WHEN edit ? 'release_year' THEN (edit ->> 'release_year')::smallint ELSE track.release_year END,
        sc_metadata = track.sc_metadata || coalesce(edit -> 'sc_metadata', '{}'),
        sc_desired = track.sc_desired || (edit - 'sc_metadata') || CASE
            WHEN edit ? 'sc_metadata' THEN jsonb_build_object('sc_metadata', coalesce(track.sc_desired -> 'sc_metadata', '{}') || (edit -> 'sc_metadata'))
            ELSE '{}'::jsonb
        END,
        sc_observation = GREATEST(track.sc_observation, seen),
        sc_mutation_observation = GREATEST(track.sc_mutation_observation, seen),
        sc_write_confirmed = false,
        updated_at = stamp
    WHERE track.sc_track_id = args ->> 'sc_track_id'
      AND track.uploader_sc_user_id = args ->> 'uploader'
      AND track.deleted_at IS NULL
    RETURNING track.id, track.urn INTO edited, edited_urn;
    IF NOT FOUND THEN
        RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', NULL::text);
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'tracks', 'pk', jsonb_build_object('id', edited))),
        'output', edited_urn);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_track_delete(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    stamp timestamptz := (args ->> 'at')::timestamptz;
    seen bigint := (args ->> 'observation')::bigint;
    deleted uuid;
    deleted_urn text;
BEGIN
    UPDATE public.tracks AS track
    SET sharing = 'private',
        sc_desired = '{}',
        deleted_at = coalesce(track.deleted_at, stamp),
        sc_observation = GREATEST(track.sc_observation, seen),
        sc_mutation_observation = GREATEST(track.sc_mutation_observation, seen),
        sc_write_confirmed = false,
        updated_at = stamp
    WHERE track.sc_track_id = args ->> 'sc_track_id'
      AND track.uploader_sc_user_id = args ->> 'uploader'
    RETURNING track.id, track.urn INTO deleted, deleted_urn;
    IF NOT FOUND THEN
        RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', NULL::text);
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'tracks', 'pk', jsonb_build_object('id', deleted))),
        'output', deleted_urn);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_playlist_edit(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    stamp timestamptz := (args ->> 'at')::timestamptz;
    seen bigint := (args ->> 'observation')::bigint;
    edit jsonb := args -> 'desired';
    edited_urn text;
BEGIN
    UPDATE public.playlists AS playlist
    SET title = CASE WHEN edit ? 'title' THEN edit ->> 'title' ELSE playlist.title END,
        title_normalized = CASE WHEN edit ? 'title_normalized' THEN edit ->> 'title_normalized' ELSE playlist.title_normalized END,
        description = CASE WHEN edit ? 'description' THEN edit ->> 'description' ELSE playlist.description END,
        genre = CASE WHEN edit ? 'genre' THEN edit ->> 'genre' ELSE playlist.genre END,
        tags = CASE WHEN edit ? 'tags' THEN ARRAY(SELECT jsonb_array_elements_text(edit -> 'tags')) ELSE playlist.tags END,
        sharing = CASE WHEN edit ? 'sharing' THEN edit ->> 'sharing' ELSE playlist.sharing END,
        label_name = CASE WHEN edit ? 'label_name' THEN edit ->> 'label_name' ELSE playlist.label_name END,
        permalink_url = CASE WHEN edit ? 'permalink_url' THEN edit ->> 'permalink_url' ELSE playlist.permalink_url END,
        playlist_type = CASE WHEN edit ? 'playlist_type' THEN edit ->> 'playlist_type' ELSE playlist.playlist_type END,
        release_date = CASE WHEN edit ? 'release_date' THEN (edit ->> 'release_date')::date ELSE playlist.release_date END,
        release_year = CASE WHEN edit ? 'release_year' THEN (edit ->> 'release_year')::smallint ELSE playlist.release_year END,
        sc_metadata = playlist.sc_metadata || coalesce(edit -> 'sc_metadata', '{}'),
        sc_desired = playlist.sc_desired || edit || CASE
            WHEN edit ? 'sc_metadata' THEN jsonb_build_object('sc_metadata', coalesce(playlist.sc_desired -> 'sc_metadata', '{}') || (edit -> 'sc_metadata'))
            ELSE '{}'::jsonb
        END,
        sc_observation = GREATEST(playlist.sc_observation, seen),
        sc_mutation_observation = GREATEST(playlist.sc_mutation_observation, seen),
        sc_write_confirmed = false,
        updated_at = stamp
    WHERE playlist.urn = args ->> 'urn'
      AND playlist.owner_sc_user_id = args ->> 'owner'
      AND playlist.deleted_at IS NULL
    RETURNING playlist.urn INTO edited_urn;
    IF NOT FOUND THEN
        RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', NULL::text);
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'playlists', 'pk', jsonb_build_object('urn', edited_urn))),
        'output', edited_urn);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_playlist_delete(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    stamp timestamptz := (args ->> 'at')::timestamptz;
    seen bigint := (args ->> 'observation')::bigint;
    target text := args ->> 'urn';
    touched jsonb := '[]';
BEGIN
    PERFORM public.scd_lock_row('playlist_membership_state', jsonb_build_object('playlist_urn', target));
    PERFORM public.scd_lock_row('playlists', jsonb_build_object('urn', target));
    IF NOT EXISTS (
        SELECT 1 FROM public.playlists AS playlist
        WHERE playlist.urn = target AND playlist.owner_sc_user_id = args ->> 'owner'
    ) THEN
        RETURN jsonb_build_object('touched', touched, 'output', NULL::text);
    END IF;
    UPDATE public.playlist_membership_state
    SET next_reconcile_at = NULL,
        reconcile_generation = reconcile_generation + 1,
        updated_at = stamp
    WHERE playlist_urn = target;
    IF FOUND THEN
        touched := jsonb_build_array(jsonb_build_object(
            'tbl', 'playlist_membership_state', 'pk', jsonb_build_object('playlist_urn', target)));
    END IF;
    UPDATE public.playlists AS playlist
    SET sharing = 'private',
        sc_desired = '{}',
        deleted_at = coalesce(playlist.deleted_at, stamp),
        sc_observation = GREATEST(playlist.sc_observation, seen),
        sc_mutation_observation = GREATEST(playlist.sc_mutation_observation, seen),
        sc_write_confirmed = false,
        updated_at = stamp
    WHERE playlist.urn = target;
    RETURN jsonb_build_object(
        'touched', touched || jsonb_build_array(jsonb_build_object('tbl', 'playlists', 'pk', jsonb_build_object('urn', target))),
        'output', target);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_owned_insert(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    stamp timestamptz := (args ->> 'at')::timestamptz;
BEGIN
    INSERT INTO public.user_owned_tracks (user_id, sc_track_id, progress, synced_at, created_at)
    SELECT args ->> 'user_id', args ->> 'sc_track_id', false, stamp, stamp
    WHERE NOT EXISTS (
        SELECT 1 FROM public.user_owned_tracks AS existing
        WHERE existing.user_id IN (SELECT jsonb_array_elements_text(args -> 'variants'))
          AND existing.sc_track_id = args ->> 'sc_track_id'
    )
    ON CONFLICT (user_id, sc_track_id) DO NOTHING;
    IF NOT FOUND THEN
        RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', false);
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'user_owned_tracks',
            'pk', jsonb_build_object('user_id', args ->> 'user_id', 'sc_track_id', args ->> 'sc_track_id'))),
        'output', true);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_owned_delete(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    touched jsonb;
BEGIN
    IF args ->> 'entity' = 'track' THEN
        WITH removed AS (
            DELETE FROM public.user_owned_tracks AS owned
            WHERE owned.user_id IN (SELECT jsonb_array_elements_text(args -> 'variants'))
              AND owned.sc_track_id = args ->> 'sc_track_id'
            RETURNING owned.user_id, owned.sc_track_id
        )
        SELECT coalesce(jsonb_agg(jsonb_build_object('tbl', 'user_owned_tracks',
                   'pk', jsonb_build_object('user_id', removed.user_id, 'sc_track_id', removed.sc_track_id))
                   ORDER BY removed.user_id), '[]')
        INTO touched
        FROM removed;
    ELSE
        WITH removed AS (
            DELETE FROM public.user_owned_playlists AS owned
            WHERE owned.user_id IN (SELECT jsonb_array_elements_text(args -> 'variants'))
              AND owned.playlist_urn = args ->> 'playlist_urn'
            RETURNING owned.user_id, owned.playlist_urn
        )
        SELECT coalesce(jsonb_agg(jsonb_build_object('tbl', 'user_owned_playlists',
                   'pk', jsonb_build_object('user_id', removed.user_id, 'playlist_urn', removed.playlist_urn))
                   ORDER BY removed.user_id), '[]')
        INTO touched
        FROM removed;
    END IF;
    RETURN jsonb_build_object('touched', touched, 'output', jsonb_array_length(touched));
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_comment_post(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
BEGIN
    INSERT INTO public.track_comments (id, sc_track_id, user_urn, body, track_position_ms, created_at)
    VALUES ((args ->> 'id')::uuid, args ->> 'sc_track_id', args ->> 'user_urn', args ->> 'body',
            (args ->> 'track_position_ms')::bigint, (args ->> 'at')::timestamptz)
    ON CONFLICT (id) DO NOTHING;
    IF NOT FOUND THEN
        RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', false);
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'track_comments', 'pk', jsonb_build_object('id', args ->> 'id'))),
        'output', true);
END
$f$;

CREATE OR REPLACE FUNCTION public.playlist_membership_apply(list text[], operation jsonb)
RETURNS text[]
LANGUAGE plpgsql
IMMUTABLE
SET search_path = public, pg_catalog
AS $f$
DECLARE
    track text := operation ->> 'track_id';
    slot integer;
    reordered text[];
BEGIN
    CASE operation ->> 'kind'
    WHEN 'remove' THEN
        RETURN array_remove(list, track);
    WHEN 'reorder' THEN
        SELECT array_agg(coalesce(renamed.track_id, member.track_id) ORDER BY member.position)
        INTO reordered
        FROM unnest(list) WITH ORDINALITY AS member(track_id, position)
        LEFT JOIN (
            SELECT slots.position, names.track_id
            FROM (
                SELECT kept.position, row_number() OVER (ORDER BY kept.position) AS rank
                FROM unnest(list) WITH ORDINALITY AS kept(track_id, position)
                WHERE kept.track_id IN (SELECT jsonb_array_elements_text(operation -> 'ordered_track_ids'))
            ) AS slots
            JOIN (
                SELECT named.track_id, row_number() OVER (ORDER BY named.position) AS rank
                FROM jsonb_array_elements_text(operation -> 'ordered_track_ids') WITH ORDINALITY AS named(track_id, position)
                WHERE named.track_id IN (SELECT unnest(list))
            ) AS names ON names.rank = slots.rank
        ) AS renamed ON renamed.position = member.position;
        RETURN coalesce(reordered, '{}');
    WHEN 'move' THEN
        IF array_position(list, track) IS NULL THEN
            RETURN list;
        END IF;
        list := array_remove(list, track);
    ELSE
        IF array_position(list, track) IS NOT NULL THEN
            RETURN list;
        END IF;
    END CASE;
    slot := CASE operation ->> 'boundary'
        WHEN 'front' THEN 0
        WHEN 'back' THEN cardinality(list)
        ELSE coalesce(
            array_position(list, operation ->> 'left_anchor_track_id'),
            array_position(list, operation ->> 'right_anchor_track_id') - 1,
            cardinality(list))
    END;
    RETURN list[1:slot] || track || list[slot + 1:];
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_playlist_membership_op(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    stamp timestamptz := (args ->> 'at')::timestamptz;
    target text := args ->> 'playlist_urn';
    operations jsonb := coalesce(args -> 'operations', '[]');
    total integer := jsonb_array_length(coalesce(args -> 'operations', '[]'));
    state record;
    base bigint;
    replayed record;
    refusal text;
    old_tracks text[];
    new_tracks text[];
    operation jsonb;
    dropped text[];
    positions integer[];
    touched jsonb := '[]';
BEGIN
    INSERT INTO public.playlist_membership_state (
        playlist_urn, projection_track_count, sync_status, next_reconcile_at, updated_at
    )
    SELECT playlist.urn,
           (SELECT count(*)::integer FROM public.playlist_track_projection AS projection WHERE projection.playlist_urn = playlist.urn),
           'unhydrated',
           stamp,
           stamp
    FROM public.playlists AS playlist
    WHERE playlist.urn = target AND playlist.deleted_at IS NULL
    ON CONFLICT (playlist_urn) DO NOTHING;
    IF FOUND THEN
        touched := jsonb_build_array(jsonb_build_object(
            'tbl', 'playlist_membership_state', 'pk', jsonb_build_object('playlist_urn', target)));
    END IF;
    PERFORM public.scd_lock_row('playlist_membership_state', jsonb_build_object('playlist_urn', target));
    SELECT s.baseline_generation, s.baseline_observation_id, s.projection_revision,
           s.last_operation_sequence, s.committed_operation_sequence
    INTO state
    FROM public.playlist_membership_state AS s
    WHERE s.playlist_urn = target;
    IF NOT FOUND THEN
        RETURN jsonb_build_object('touched', touched, 'output', jsonb_build_object(
            'status', 'refused', 'code', 'playlist_not_found', 'appended', 0, 'projection_revision', 0, 'added', '[]'::jsonb));
    END IF;
    IF total = 0 THEN
        RETURN jsonb_build_object('touched', touched, 'output', jsonb_build_object(
            'status', 'tracked', 'code', NULL::text, 'appended', 0, 'projection_revision', state.projection_revision, 'added', '[]'::jsonb));
    END IF;

    SELECT o.outcome, o.conflict_code INTO replayed
    FROM public.playlist_membership_operations AS o
    WHERE o.playlist_urn = target
      AND o.idempotency_key = (operations -> 0 ->> 'idempotency_key')::uuid;
    IF FOUND THEN
        RETURN jsonb_build_object('touched', touched, 'output', jsonb_build_object(
            'status', CASE WHEN replayed.outcome = 'conflict' THEN 'refused' ELSE 'replayed' END,
            'code', CASE WHEN replayed.outcome = 'conflict' THEN replayed.conflict_code END,
            'appended', 0, 'projection_revision', state.projection_revision, 'added', '[]'::jsonb));
    END IF;

    refusal := CASE
        WHEN NOT EXISTS (
            SELECT 1 FROM public.playlists AS playlist
            WHERE playlist.urn = target AND playlist.deleted_at IS NULL
              AND (playlist.owner_sc_user_id = args ->> 'actor' OR EXISTS (
                  SELECT 1 FROM public.user_owned_playlists AS owned
                  WHERE owned.user_id IN (SELECT jsonb_array_elements_text(args -> 'variants'))
                    AND owned.playlist_urn = playlist.urn))
        ) THEN 'playlist_not_found'
        WHEN (args ->> 'expected_projection_revision')::bigint IS DISTINCT FROM state.projection_revision
         AND args ->> 'expected_projection_revision' IS NOT NULL THEN 'playlist_revision_conflict'
        WHEN EXISTS (
            SELECT 1 FROM jsonb_array_elements(operations) AS submitted
            WHERE submitted ->> 'kind' = 'add'
              AND NOT EXISTS (
                  SELECT 1 FROM public.tracks AS track
                  WHERE track.sc_track_id = submitted ->> 'track_id' AND track.deleted_at IS NULL)
        ) THEN 'playlist_track_not_in_catalog'
    END;

    SELECT GREATEST(state.last_operation_sequence, coalesce(max(o.sequence), 0)) INTO base
    FROM public.playlist_membership_operations AS o
    WHERE o.playlist_urn = target;

    IF refusal IS NULL THEN
        old_tracks := ARRAY(
            SELECT projection.sc_track_id FROM public.playlist_track_projection AS projection
            WHERE projection.playlist_urn = target ORDER BY projection.position);
        new_tracks := old_tracks;
        FOR operation IN SELECT e.value FROM jsonb_array_elements(operations) AS e LOOP
            new_tracks := public.playlist_membership_apply(new_tracks, operation);
        END LOOP;
    END IF;

    UPDATE public.playlist_membership_state
    SET last_operation_sequence = base + total,
        committed_operation_sequence = CASE
            WHEN refusal IS NOT NULL AND committed_operation_sequence = state.last_operation_sequence
            THEN base + total
            ELSE committed_operation_sequence
        END,
        projection_revision = CASE WHEN refusal IS NULL THEN state.projection_revision + total ELSE projection_revision END,
        projection_track_count = CASE WHEN refusal IS NULL THEN cardinality(new_tracks) ELSE projection_track_count END,
        sync_status = CASE WHEN refusal IS NULL THEN 'pending' ELSE sync_status END,
        conflict_code = CASE WHEN refusal IS NULL THEN NULL ELSE conflict_code END,
        candidate_fingerprint = CASE WHEN refusal IS NULL THEN NULL ELSE candidate_fingerprint END,
        next_reconcile_at = CASE WHEN refusal IS NULL THEN stamp + interval '30 seconds' ELSE next_reconcile_at END,
        updated_at = stamp
    WHERE playlist_urn = target;
    IF touched = '[]'::jsonb THEN
        touched := jsonb_build_array(jsonb_build_object(
            'tbl', 'playlist_membership_state', 'pk', jsonb_build_object('playlist_urn', target)));
    END IF;

    INSERT INTO public.playlist_membership_operations (
        operation_id, playlist_urn, sequence, actor_sc_user_id, idempotency_key, request_fingerprint,
        base_baseline_generation, base_observation_id, expected_projection_revision, accepted_projection_revision,
        kind, track_id, left_anchor_track_id, right_anchor_track_id, boundary, ordered_track_ids,
        outcome, conflict_code, created_at, resolved_at
    )
    SELECT (submitted.value ->> 'operation_id')::uuid,
           target,
           base + submitted.ordinality,
           args ->> 'actor',
           (submitted.value ->> 'idempotency_key')::uuid,
           decode(submitted.value ->> 'fingerprint', 'hex'),
           state.baseline_generation,
           state.baseline_observation_id,
           state.projection_revision + submitted.ordinality - 1,
           state.projection_revision + submitted.ordinality,
           submitted.value ->> 'kind',
           submitted.value ->> 'track_id',
           submitted.value ->> 'left_anchor_track_id',
           submitted.value ->> 'right_anchor_track_id',
           submitted.value ->> 'boundary',
           CASE WHEN jsonb_typeof(submitted.value -> 'ordered_track_ids') = 'array'
                THEN ARRAY(SELECT jsonb_array_elements_text(submitted.value -> 'ordered_track_ids')) END,
           CASE WHEN refusal IS NULL THEN 'pending' ELSE 'conflict' END,
           refusal,
           stamp,
           CASE WHEN refusal IS NOT NULL THEN stamp END
    FROM jsonb_array_elements(operations) WITH ORDINALITY AS submitted(value, ordinality);
    touched := touched || (
        SELECT jsonb_agg(jsonb_build_object('tbl', 'playlist_membership_operations',
                   'pk', jsonb_build_object('operation_id', submitted ->> 'operation_id')))
        FROM jsonb_array_elements(operations) AS submitted);

    IF refusal IS NOT NULL THEN
        RETURN jsonb_build_object('touched', touched, 'output', jsonb_build_object(
            'status', 'refused', 'code', refusal, 'appended', 0,
            'projection_revision', state.projection_revision, 'added', '[]'::jsonb));
    END IF;

    IF new_tracks = old_tracks THEN
        positions := '{}';
    ELSIF cardinality(new_tracks) > cardinality(old_tracks) AND new_tracks[1:cardinality(old_tracks)] = old_tracks THEN
        WITH appended AS (
            INSERT INTO public.playlist_track_projection (playlist_urn, position, sc_track_id)
            SELECT target,
                   (SELECT coalesce(max(existing.position), -1) FROM public.playlist_track_projection AS existing
                    WHERE existing.playlist_urn = target)::integer + entry.ordinality::integer,
                   entry.sc_track_id
            FROM unnest(new_tracks[cardinality(old_tracks) + 1:]) WITH ORDINALITY AS entry(sc_track_id, ordinality)
            RETURNING position
        )
        SELECT coalesce(array_agg(appended.position), '{}') INTO positions FROM appended;
    ELSE
        dropped := ARRAY(SELECT member FROM unnest(old_tracks) AS member WHERE member NOT IN (SELECT unnest(new_tracks)));
        IF cardinality(new_tracks) < cardinality(old_tracks)
           AND ARRAY(SELECT member.track_id FROM unnest(old_tracks) WITH ORDINALITY AS member(track_id, position)
                     WHERE member.track_id IN (SELECT unnest(new_tracks)) ORDER BY member.position) = new_tracks THEN
            WITH removed AS (
                DELETE FROM public.playlist_track_projection AS projection
                WHERE projection.playlist_urn = target AND projection.sc_track_id = ANY (dropped)
                RETURNING projection.position
            )
            SELECT coalesce(array_agg(removed.position), '{}') INTO positions FROM removed;
        ELSE
            WITH removed AS (
                DELETE FROM public.playlist_track_projection AS projection
                WHERE projection.playlist_urn = target
                RETURNING projection.position
            )
            SELECT coalesce(array_agg(removed.position), '{}') INTO positions FROM removed;
            WITH inserted AS (
                INSERT INTO public.playlist_track_projection (playlist_urn, position, sc_track_id)
                SELECT target, (entry.ordinality - 1)::integer, entry.sc_track_id
                FROM unnest(new_tracks) WITH ORDINALITY AS entry(sc_track_id, ordinality)
                RETURNING position
            )
            SELECT positions || coalesce(array_agg(inserted.position), '{}') INTO positions FROM inserted;
        END IF;
    END IF;
    touched := touched || (
        SELECT coalesce(jsonb_agg(jsonb_build_object('tbl', 'playlist_track_projection',
                   'pk', jsonb_build_object('playlist_urn', target, 'position', slot.position)) ORDER BY slot.position), '[]')
        FROM (SELECT DISTINCT unnest(positions) AS position) AS slot);

    RETURN jsonb_build_object('touched', touched, 'output', jsonb_build_object(
        'status', 'applied', 'code', NULL::text, 'appended', total,
        'projection_revision', state.projection_revision + total,
        'added', to_jsonb(ARRAY(
            SELECT member.track_id FROM unnest(new_tracks) WITH ORDINALITY AS member(track_id, position)
            WHERE member.track_id NOT IN (SELECT unnest(old_tracks)) ORDER BY member.position))));
END
$f$;

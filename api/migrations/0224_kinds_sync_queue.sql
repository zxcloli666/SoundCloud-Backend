SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_cmd_sync_queue_revive(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    users text[] := ARRAY(SELECT jsonb_array_elements_text(args -> 'user_ids'));
    dropped text[] := ARRAY(SELECT jsonb_array_elements_text(args -> 'dropped_targets'));
    target text := args ->> 'target_urn';
    stamp timestamptz := (args ->> 'at')::timestamptz;
    r record;
    n bigint := 0;
    touched jsonb := '[]';
BEGIN
    FOR r IN
        UPDATE public.sync_queue AS queued
        SET dead = false,
            failed_at = NULL,
            retry_count = 0,
            last_error = NULL,
            next_run_at = stamp
        WHERE queued.user_id = ANY (users)
          AND queued.dead
          AND queued.failed_at < stamp - interval '2 minutes'
          AND queued.failed_at > stamp - interval '30 days'
          AND queued.action_type NOT IN (
              'comment', 'like_track', 'unlike_track', 'like_playlist', 'unlike_playlist',
              'follow_user', 'unfollow_user', 'playlist_membership'
          )
          AND (target IS NULL OR queued.target_urn = target)
        RETURNING queued.id
    LOOP
        n := n + 1;
        touched := touched || jsonb_build_array(jsonb_build_object('tbl', 'sync_queue', 'pk', jsonb_build_object('id', r.id)));
    END LOOP;
    FOR r IN
        SELECT queued.id, queued.intent_v
        FROM public.sync_queue AS queued
        WHERE queued.user_id = ANY (users)
          AND queued.dead
          AND queued.failed_at < stamp - interval '2 minutes'
          AND queued.failed_at > stamp - interval '30 days'
          AND queued.action_type = 'playlist_membership'
          AND queued.target_urn = ANY (dropped)
          AND (target IS NULL OR queued.target_urn = target)
    LOOP
        IF public.scd_purge_intent('sync_queue', jsonb_build_object('id', r.id), r.intent_v) THEN
            n := n + 1;
            touched := touched || jsonb_build_array(jsonb_build_object('tbl', 'sync_queue', 'pk', jsonb_build_object('id', r.id)));
        END IF;
    END LOOP;
    RETURN jsonb_build_object('touched', touched, 'output', n);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_sync_queue_purge(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    least_retries integer := (args ->> 'min_retries')::integer;
    stamp timestamptz := (args ->> 'at')::timestamptz;
    r record;
    n bigint := 0;
    touched jsonb := '[]';
BEGIN
    UPDATE public.sync_queue AS queued
    SET dead = true,
        failed_at = coalesce(queued.failed_at, stamp),
        next_run_at = 'infinity'
    WHERE queued.retry_count >= least_retries
      AND queued.lease_id IS NULL
      AND queued.remote_attempted_generation IS NULL
      AND queued.remote_completed_generation IS NULL
      AND NOT queued.dead;
    FOR r IN
        SELECT queued.id, queued.intent_v
        FROM public.sync_queue AS queued
        WHERE queued.retry_count >= least_retries
          AND queued.lease_id IS NULL
          AND queued.remote_attempted_generation IS NULL
          AND queued.remote_completed_generation IS NULL
          AND queued.dead
    LOOP
        IF public.scd_purge_intent('sync_queue', jsonb_build_object('id', r.id), r.intent_v) THEN
            n := n + 1;
            touched := touched || jsonb_build_array(jsonb_build_object('tbl', 'sync_queue', 'pk', jsonb_build_object('id', r.id)));
        END IF;
    END LOOP;
    RETURN jsonb_build_object('touched', touched, 'output', n);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_connection_drop_orphan(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    connection_id uuid := (args ->> 'connection_id')::uuid;
    seen bigint;
BEGIN
    SELECT connection.token_v INTO seen
    FROM public.soundcloud_connections AS connection
    WHERE connection.id = connection_id
      AND NOT EXISTS (
          SELECT 1
          FROM public.sessions AS session
          WHERE session.soundcloud_connection_id = connection.id
      );
    IF NOT FOUND OR NOT public.scd_purge_intent('soundcloud_connections', jsonb_build_object('id', connection_id), seen) THEN
        RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', false);
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'soundcloud_connections', 'pk', jsonb_build_object('id', connection_id))),
        'output', true
    );
END
$f$;

SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_cmd_job_enqueue(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    claim jsonb := args -> 'claim';
    stamp timestamptz;
    inserted bigint;
BEGIN
    IF jsonb_typeof(claim) = 'object' THEN
        stamp := (claim ->> 'at')::timestamptz;
        UPDATE public.playlist_membership_state AS state
        SET next_reconcile_at = stamp + interval '30 seconds',
            updated_at = stamp
        WHERE state.playlist_urn = claim ->> 'playlist_urn'
          AND EXISTS (SELECT 1 FROM public.playlists p WHERE p.urn = state.playlist_urn AND p.deleted_at IS NULL)
          AND state.next_reconcile_at <= stamp;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('touched', '[]'::jsonb,
                'output', jsonb_build_object('claimed', false, 'inserted', 0));
        END IF;
    END IF;

    WITH wanted AS (
        SELECT DISTINCT ON (j.kind, j.dedup_key)
               j.id, j.kind, j.lane, j.dedup_key, j.payload, j.max_attempts, j.raise_priority,
               j.priority + CASE
                   WHEN EXISTS (
                       SELECT 1 FROM public.subscriptions AS subscription
                       WHERE subscription.user_urn = ANY (j.star_candidates)
                         AND subscription.exp_date > extract(epoch FROM now())::bigint
                   ) THEN j.star_bonus
                   ELSE 0::smallint
               END AS priority
        FROM jsonb_to_recordset(args -> 'jobs') AS j(
            id uuid, kind text, lane text, dedup_key text, payload jsonb, priority smallint,
            max_attempts smallint, raise_priority boolean, star_candidates text[], star_bonus smallint
        )
        ORDER BY j.kind, j.dedup_key, j.priority DESC
    ), raised AS (
        INSERT INTO public.background_jobs (id, kind, lane, dedup_key, payload, priority, max_attempts)
        SELECT w.id, w.kind, w.lane, w.dedup_key, w.payload, w.priority, w.max_attempts
        FROM wanted AS w
        WHERE w.raise_priority
        ON CONFLICT (kind, dedup_key) WHERE dedup_key IS NOT NULL DO UPDATE
        SET priority = EXCLUDED.priority
        WHERE background_jobs.priority < EXCLUDED.priority
        RETURNING kind, dedup_key, available_at, (xmax = 0) AS fresh
    ), kept AS (
        INSERT INTO public.background_jobs (id, kind, lane, dedup_key, payload, priority, max_attempts)
        SELECT w.id, w.kind, w.lane, w.dedup_key, w.payload, w.priority, w.max_attempts
        FROM wanted AS w
        WHERE NOT w.raise_priority
        ON CONFLICT (kind, dedup_key) WHERE dedup_key IS NOT NULL DO NOTHING
        RETURNING kind, dedup_key, available_at, true AS fresh
    ), created AS (
        SELECT * FROM raised WHERE fresh
        UNION ALL
        SELECT * FROM kept
    ), status AS (
        INSERT INTO public.job_status (kind, dedup_key, state, available_at, updated_at)
        SELECT c.kind, c.dedup_key, 'queued', c.available_at, now()
        FROM created AS c
        WHERE c.kind = ANY (ARRAY['catalog.refresh', 'catalog.collection', 'playlists.observe_shadow'])
        ON CONFLICT (kind, dedup_key) DO NOTHING
        RETURNING 1
    )
    SELECT count(*) INTO inserted FROM created;

    RETURN jsonb_build_object('touched', '[]'::jsonb,
        'output', jsonb_build_object('claimed', true, 'inserted', inserted));
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_lyrics_wake(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    track text := args ->> 'sc_track_id';
    stamp timestamptz := (args ->> 'at')::timestamptz;
    state_id uuid;
    wake record;
BEGIN
    INSERT INTO public.lyrics_lookup_state (
        track_id, sc_track_id, priority, next_run_at, input_title, input_artist, input_duration_ms,
        input_genius_song_id, input_genius_url, input_release_date, input_sc_created_at,
        wake_message_id, wake_generation, created_at, updated_at
    )
    SELECT t.id, t.sc_track_id, 0, stamp, btrim(t.title),
           COALESCE(NULLIF(btrim(t.metadata_artist), ''), NULLIF(btrim(t.uploader_username), ''), ''),
           t.duration_ms, t.genius_song_id, NULLIF(btrim(t.genius_url), ''), t.release_date, t.sc_created_at,
           public.scd_uuid('lyrics.wake', t.sc_track_id, '1'), 1, t.created_at, stamp
    FROM public.tracks AS t
    WHERE t.sc_track_id = track
      AND NOT EXISTS (
          SELECT 1 FROM public.lyrics_cache AS cache
          WHERE cache.sc_track_id = t.sc_track_id
            AND (NULLIF(btrim(cache.plain_text), '') IS NOT NULL OR NULLIF(btrim(cache.synced_lrc), '') IS NOT NULL)
      )
    ON CONFLICT (track_id) DO NOTHING;

    UPDATE public.lyrics_lookup_state AS state
    SET status = 'pending',
        generation = state.generation + 1,
        next_run_at = stamp,
        last_outcome = NULL,
        last_error = NULL,
        retry_after_at = NULL,
        wake_message_id = public.scd_uuid('lyrics.wake', state.sc_track_id, (state.generation + 1)::text),
        wake_generation = state.generation + 1,
        wake_durable_at = NULL
    WHERE state.sc_track_id = track
      AND state.claim_job_id IS NULL
      AND state.status IN ('retry', 'not_found')
      AND state.next_run_at <= stamp;

    UPDATE public.lyrics_lookup_state AS state
    SET wake_message_id = public.scd_uuid('lyrics.wake', state.sc_track_id, state.generation::text),
        wake_generation = state.generation,
        wake_durable_at = NULL
    WHERE state.sc_track_id = track
      AND state.claim_job_id IS NULL
      AND state.status = 'pending'
      AND state.wake_message_id IS NULL;

    UPDATE public.lyrics_lookup_state AS state
    SET priority = 0,
        updated_at = stamp
    WHERE state.sc_track_id = track
    RETURNING state.track_id, state.status, state.next_run_at, state.failure_streak, state.claim_job_id
    INTO wake;

    IF wake.track_id IS NULL THEN
        RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', NULL);
    END IF;
    state_id := wake.track_id;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'lyrics_lookup_state', 'pk', jsonb_build_object('track_id', state_id))),
        'output', jsonb_build_object(
            'status', wake.status, 'next_run_at', wake.next_run_at,
            'failure_streak', wake.failure_streak, 'claim_job_id', wake.claim_job_id));
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_cold_refresh_clear(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
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

CREATE OR REPLACE FUNCTION public.scd_cmd_app_token_reject(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    app uuid;
BEGIN
    UPDATE public.oauth_app_tokens
    SET expires_at = LEAST(expires_at, (args ->> 'at')::timestamptz)
    WHERE oauth_app_id = (args ->> 'oauth_app_id')::uuid
      AND generation = (args ->> 'generation')::uuid
    RETURNING oauth_app_id INTO app;
    IF app IS NULL THEN
        RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', false);
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'oauth_app_tokens', 'pk', jsonb_build_object('oauth_app_id', app))),
        'output', true);
END
$f$;

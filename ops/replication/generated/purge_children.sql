CREATE OR REPLACE FUNCTION public.scd_purge_admin_maintenance_runs(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.kind INTO p FROM public.admin_maintenance_runs t WHERE t.kind = (k ->> 'kind')::character varying(64);
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.admin_maintenance_runs t WHERE t.kind = (k ->> 'kind')::character varying(64);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_ai_resolver_budget(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.spent_on INTO p FROM public.ai_resolver_budget t WHERE t.spent_on = (k ->> 'spent_on')::date;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.ai_resolver_budget t WHERE t.spent_on = (k ->> 'spent_on')::date;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_ai_resolver_cache(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.cache_key INTO p FROM public.ai_resolver_cache t WHERE t.cache_key = (k ->> 'cache_key')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.ai_resolver_cache t WHERE t.cache_key = (k ->> 'cache_key')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_album_artists(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.album_id, t.artist_id, t.role INTO p FROM public.album_artists t WHERE t.album_id = (k ->> 'album_id')::uuid AND t.artist_id = (k ->> 'artist_id')::uuid AND t.role = (k ->> 'role')::character varying(16);
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.album_artists t WHERE t.album_id = (k ->> 'album_id')::uuid AND t.artist_id = (k ->> 'artist_id')::uuid AND t.role = (k ->> 'role')::character varying(16);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_album_tracks(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.album_id, t.track_id INTO p FROM public.album_tracks t WHERE t.album_id = (k ->> 'album_id')::uuid AND t.track_id = (k ->> 'track_id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.album_tracks t WHERE t.album_id = (k ->> 'album_id')::uuid AND t.track_id = (k ->> 'track_id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_albums(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.id INTO p FROM public.albums t WHERE t.id = (k ->> 'id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.album_artists c WHERE c.album_id = p.id;
    DELETE FROM public.album_tracks c WHERE c.album_id = p.id;
    UPDATE public.tracks c SET album_id = NULL WHERE c.album_id = p.id;
    DELETE FROM public.wanted_track_albums c WHERE c.album_id = p.id;
    DELETE FROM public.albums t WHERE t.id = (k ->> 'id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_artist_attribution_revalidation(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.singleton INTO p FROM public.artist_attribution_revalidation t WHERE t.singleton = (k ->> 'singleton')::boolean;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.artist_attribution_revalidation t WHERE t.singleton = (k ->> 'singleton')::boolean;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_artist_colike(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.a_id, t.b_id INTO p FROM public.artist_colike t WHERE t.a_id = (k ->> 'a_id')::uuid AND t.b_id = (k ->> 'b_id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.artist_colike t WHERE t.a_id = (k ->> 'a_id')::uuid AND t.b_id = (k ->> 'b_id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_artist_coplay(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.a_id, t.b_id INTO p FROM public.artist_coplay t WHERE t.a_id = (k ->> 'a_id')::uuid AND t.b_id = (k ->> 'b_id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.artist_coplay t WHERE t.a_id = (k ->> 'a_id')::uuid AND t.b_id = (k ->> 'b_id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_artist_sc_accounts(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.artist_id, t.sc_user_id INTO p FROM public.artist_sc_accounts t WHERE t.artist_id = (k ->> 'artist_id')::uuid AND t.sc_user_id = (k ->> 'sc_user_id')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.artist_sc_accounts t WHERE t.artist_id = (k ->> 'artist_id')::uuid AND t.sc_user_id = (k ->> 'sc_user_id')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_artist_socials(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.artist_id, t.url INTO p FROM public.artist_socials t WHERE t.artist_id = (k ->> 'artist_id')::uuid AND t.url = (k ->> 'url')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.artist_socials t WHERE t.artist_id = (k ->> 'artist_id')::uuid AND t.url = (k ->> 'url')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_artists(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.id INTO p FROM public.artists t WHERE t.id = (k ->> 'id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.album_artists c WHERE c.artist_id = p.id;
    UPDATE public.albums c SET primary_artist_id = NULL WHERE c.primary_artist_id = p.id;
    DELETE FROM public.artist_colike c WHERE c.a_id = p.id;
    DELETE FROM public.artist_colike c WHERE c.b_id = p.id;
    DELETE FROM public.artist_coplay c WHERE c.a_id = p.id;
    DELETE FROM public.artist_coplay c WHERE c.b_id = p.id;
    DELETE FROM public.artist_sc_accounts c WHERE c.artist_id = p.id;
    DELETE FROM public.artist_socials c WHERE c.artist_id = p.id;
    UPDATE public.artists c SET merged_into = NULL WHERE c.merged_into = p.id;
    DELETE FROM public.track_artist_blocks c WHERE c.artist_id = p.id;
    DELETE FROM public.track_artists c WHERE c.artist_id = p.id;
    UPDATE public.tracks c SET cover_of_artist_id = NULL WHERE c.cover_of_artist_id = p.id;
    UPDATE public.tracks c SET primary_artist_id = NULL WHERE c.primary_artist_id = p.id;
    DELETE FROM public.wanted_track_artists c WHERE c.artist_id = p.id;
    UPDATE public.wanted_tracks c SET primary_artist_id = NULL WHERE c.primary_artist_id = p.id;
    DELETE FROM public.artists t WHERE t.id = (k ->> 'id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_audio_index_wire_state(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.sc_track_id INTO p FROM public.audio_index_wire_state t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.audio_index_wire_state t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_background_job_enqueues(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.id INTO p FROM public.background_job_enqueues t WHERE t.id = (k ->> 'id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.background_job_enqueues t WHERE t.id = (k ->> 'id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_background_job_failures(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.generation, t.id INTO p FROM public.background_job_failures t WHERE t.id = (k ->> 'id')::uuid AND t.generation = (k ->> 'generation')::bigint;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.background_job_failures t WHERE t.id = (k ->> 'id')::uuid AND t.generation = (k ->> 'generation')::bigint;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_background_jobs(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.id INTO p FROM public.background_jobs t WHERE t.id = (k ->> 'id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.background_jobs t WHERE t.id = (k ->> 'id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_background_schedules(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.kind INTO p FROM public.background_schedules t WHERE t.kind = (k ->> 'kind')::character varying(96);
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.background_schedules t WHERE t.kind = (k ->> 'kind')::character varying(96);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_catalog_audience(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.relation, t.subject_urn, t.user_urn INTO p FROM public.catalog_audience t WHERE t.subject_urn = (k ->> 'subject_urn')::text AND t.relation = (k ->> 'relation')::text AND t.user_urn = (k ->> 'user_urn')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.catalog_audience t WHERE t.subject_urn = (k ->> 'subject_urn')::text AND t.relation = (k ->> 'relation')::text AND t.user_urn = (k ->> 'user_urn')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_catalog_collection_cursors(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.collection, t.cursor, t.scope, t.subject_id INTO p FROM public.catalog_collection_cursors t WHERE t.subject_id = (k ->> 'subject_id')::text AND t.collection = (k ->> 'collection')::text AND t.scope = (k ->> 'scope')::text AND t.cursor = (k ->> 'cursor')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.catalog_collection_cursors t WHERE t.subject_id = (k ->> 'subject_id')::text AND t.collection = (k ->> 'collection')::text AND t.scope = (k ->> 'scope')::text AND t.cursor = (k ->> 'cursor')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_catalog_collection_seen(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.collection, t.entity_key, t.scope, t.subject_id INTO p FROM public.catalog_collection_seen t WHERE t.subject_id = (k ->> 'subject_id')::text AND t.collection = (k ->> 'collection')::text AND t.scope = (k ->> 'scope')::text AND t.entity_key = (k ->> 'entity_key')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.catalog_collection_seen t WHERE t.subject_id = (k ->> 'subject_id')::text AND t.collection = (k ->> 'collection')::text AND t.scope = (k ->> 'scope')::text AND t.entity_key = (k ->> 'entity_key')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_catalog_collection_sync(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.collection, t.scope, t.subject_id INTO p FROM public.catalog_collection_sync t WHERE t.subject_id = (k ->> 'subject_id')::text AND t.collection = (k ->> 'collection')::text AND t.scope = (k ->> 'scope')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.catalog_collection_cursors c WHERE (c.subject_id, c.collection, c.scope) = (p.subject_id, p.collection, p.scope);
    DELETE FROM public.catalog_collection_seen c WHERE (c.subject_id, c.collection, c.scope) = (p.subject_id, p.collection, p.scope);
    DELETE FROM public.catalog_collection_sync t WHERE t.subject_id = (k ->> 'subject_id')::text AND t.collection = (k ->> 'collection')::text AND t.scope = (k ->> 'scope')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_catalog_merge_state(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.singleton INTO p FROM public.catalog_merge_state t WHERE t.singleton = (k ->> 'singleton')::boolean;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.catalog_merge_state t WHERE t.singleton = (k ->> 'singleton')::boolean;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_catalog_work_links(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.track_id, t.wanted_track_id INTO p FROM public.catalog_work_links t WHERE t.wanted_track_id = (k ->> 'wanted_track_id')::uuid AND t.track_id = (k ->> 'track_id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.catalog_work_links t WHERE t.wanted_track_id = (k ->> 'wanted_track_id')::uuid AND t.track_id = (k ->> 'track_id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_cmd_dead(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.id, t.node INTO p FROM public.cmd_dead t WHERE t.node = (k ->> 'node')::text AND t.id = (k ->> 'id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.cmd_dead t WHERE t.node = (k ->> 'node')::text AND t.id = (k ->> 'id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_discover_tag_counts(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.tag INTO p FROM public.discover_tag_counts t WHERE t.tag = (k ->> 'tag')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.discover_tag_counts t WHERE t.tag = (k ->> 'tag')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_enrich_calibration(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.raw_bin_high, t.raw_bin_low, t.source INTO p FROM public.enrich_calibration t WHERE t.source = (k ->> 'source')::character varying(16) AND t.raw_bin_low = (k ->> 'raw_bin_low')::real AND t.raw_bin_high = (k ->> 'raw_bin_high')::real;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.enrich_calibration t WHERE t.source = (k ->> 'source')::character varying(16) AND t.raw_bin_low = (k ->> 'raw_bin_low')::real AND t.raw_bin_high = (k ->> 'raw_bin_high')::real;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_job_status(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.dedup_key, t.kind INTO p FROM public.job_status t WHERE t.kind = (k ->> 'kind')::text AND t.dedup_key = (k ->> 'dedup_key')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.job_status t WHERE t.kind = (k ->> 'kind')::text AND t.dedup_key = (k ->> 'dedup_key')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_lyrics_cache(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.sc_track_id INTO p FROM public.lyrics_cache t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.lyrics_cache t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_lyrics_embedding_wire_state(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.sc_track_id INTO p FROM public.lyrics_embedding_wire_state t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.lyrics_embedding_wire_state t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_lyrics_lookup_backfill_state(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.singleton INTO p FROM public.lyrics_lookup_backfill_state t WHERE t.singleton = (k ->> 'singleton')::boolean;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.lyrics_lookup_backfill_state t WHERE t.singleton = (k ->> 'singleton')::boolean;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_lyrics_lookup_state(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.track_id INTO p FROM public.lyrics_lookup_state t WHERE t.track_id = (k ->> 'track_id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.lyrics_lookup_state t WHERE t.track_id = (k ->> 'track_id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_oauth_app_token_issuance_reservations(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.id INTO p FROM public.oauth_app_token_issuance_reservations t WHERE t.id = (k ->> 'id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.oauth_app_token_issuance_reservations t WHERE t.id = (k ->> 'id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_oauth_app_token_refresh_state(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.oauth_app_id INTO p FROM public.oauth_app_token_refresh_state t WHERE t.oauth_app_id = (k ->> 'oauth_app_id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.oauth_app_token_refresh_state t WHERE t.oauth_app_id = (k ->> 'oauth_app_id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_oauth_app_tokens(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.oauth_app_id INTO p FROM public.oauth_app_tokens t WHERE t.oauth_app_id = (k ->> 'oauth_app_id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.oauth_app_tokens t WHERE t.oauth_app_id = (k ->> 'oauth_app_id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_pipeline_event_receipts(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.consumer, t.event_published_at, t.stream, t.stream_sequence INTO p FROM public.pipeline_event_receipts t WHERE t.consumer = (k ->> 'consumer')::character varying(96) AND t.stream = (k ->> 'stream')::character varying(128) AND t.stream_sequence = (k ->> 'stream_sequence')::bigint AND t.event_published_at = (k ->> 'event_published_at')::timestamp with time zone;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.pipeline_event_receipts t WHERE t.consumer = (k ->> 'consumer')::character varying(96) AND t.stream = (k ->> 'stream')::character varying(128) AND t.stream_sequence = (k ->> 'stream_sequence')::bigint AND t.event_published_at = (k ->> 'event_published_at')::timestamp with time zone;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_playlist_legacy_membership_intents(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.archive_id INTO p FROM public.playlist_legacy_membership_intents t WHERE t.archive_id = (k ->> 'archive_id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.playlist_legacy_membership_intents t WHERE t.archive_id = (k ->> 'archive_id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_playlist_membership_operations(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.operation_id INTO p FROM public.playlist_membership_operations t WHERE t.operation_id = (k ->> 'operation_id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.playlist_membership_operations t WHERE t.operation_id = (k ->> 'operation_id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_playlist_membership_state(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.playlist_urn INTO p FROM public.playlist_membership_state t WHERE t.playlist_urn = (k ->> 'playlist_urn')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.playlist_membership_operations c WHERE c.playlist_urn = p.playlist_urn;
    DELETE FROM public.playlist_reconcile_runs c WHERE c.playlist_urn = p.playlist_urn;
    DELETE FROM public.playlist_membership_state t WHERE t.playlist_urn = (k ->> 'playlist_urn')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_playlist_reconcile_runs(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.id INTO p FROM public.playlist_reconcile_runs t WHERE t.id = (k ->> 'id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.playlist_reconcile_runs t WHERE t.id = (k ->> 'id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_playlist_remote_observations(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.id, t.playlist_urn INTO p FROM public.playlist_remote_observations t WHERE t.id = (k ->> 'id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    UPDATE public.playlist_membership_operations c SET base_observation_id = NULL WHERE (c.playlist_urn, c.base_observation_id) = (p.playlist_urn, p.id);
    UPDATE public.playlist_membership_state c SET baseline_observation_id = NULL WHERE (c.playlist_urn, c.baseline_observation_id) = (p.playlist_urn, p.id);
    UPDATE public.playlist_membership_state c SET latest_observation_id = NULL WHERE (c.playlist_urn, c.latest_observation_id) = (p.playlist_urn, p.id);
    UPDATE public.playlist_reconcile_runs c SET observation_id = NULL WHERE (c.playlist_urn, c.observation_id) = (p.playlist_urn, p.id);
    DELETE FROM public.playlist_remote_observations t WHERE t.id = (k ->> 'id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_playlist_remote_snapshot_tracks(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t."position", t.snapshot_id INTO p FROM public.playlist_remote_snapshot_tracks t WHERE t.snapshot_id = (k ->> 'snapshot_id')::uuid AND t."position" = (k ->> 'position')::integer;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.playlist_remote_snapshot_tracks t WHERE t.snapshot_id = (k ->> 'snapshot_id')::uuid AND t."position" = (k ->> 'position')::integer;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_playlist_remote_snapshots(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.id, t.playlist_urn INTO p FROM public.playlist_remote_snapshots t WHERE t.id = (k ->> 'id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    UPDATE public.playlist_legacy_membership_intents c SET local_snapshot_id = NULL WHERE c.local_snapshot_id = p.id;
    UPDATE public.playlist_remote_observations c SET snapshot_id = NULL WHERE (c.playlist_urn, c.snapshot_id) = (p.playlist_urn, p.id);
    DELETE FROM public.playlist_remote_snapshot_tracks c WHERE c.snapshot_id = p.id;
    DELETE FROM public.playlist_remote_snapshots t WHERE t.id = (k ->> 'id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_playlist_sweep_cursor(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.id INTO p FROM public.playlist_sweep_cursor t WHERE t.id = (k ->> 'id')::boolean;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.playlist_sweep_cursor t WHERE t.id = (k ->> 'id')::boolean;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_playlist_track_projection(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.playlist_urn, t."position" INTO p FROM public.playlist_track_projection t WHERE t.playlist_urn = (k ->> 'playlist_urn')::text AND t."position" = (k ->> 'position')::integer;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.playlist_track_projection t WHERE t.playlist_urn = (k ->> 'playlist_urn')::text AND t."position" = (k ->> 'position')::integer;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_playlists(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
    r record;
BEGIN
    SELECT t.urn INTO p FROM public.playlists t WHERE t.urn = (k ->> 'urn')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    FOR r IN SELECT c.playlist_urn FROM public.playlist_membership_state c WHERE c.playlist_urn = p.urn LOOP
        PERFORM public.scd_purge_playlist_membership_state(jsonb_build_object('playlist_urn', r.playlist_urn));
    END LOOP;
    FOR r IN SELECT c.id FROM public.playlist_remote_observations c WHERE c.playlist_urn = p.urn LOOP
        PERFORM public.scd_purge_playlist_remote_observations(jsonb_build_object('id', r.id));
    END LOOP;
    FOR r IN SELECT c.id FROM public.playlist_remote_snapshots c WHERE c.playlist_urn = p.urn LOOP
        PERFORM public.scd_purge_playlist_remote_snapshots(jsonb_build_object('id', r.id));
    END LOOP;
    DELETE FROM public.playlist_track_projection c WHERE c.playlist_urn = p.urn;
    DELETE FROM public.playlists t WHERE t.urn = (k ->> 'urn')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_recommendation_quality_models(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.version INTO p FROM public.recommendation_quality_models t WHERE t.version = (k ->> 'version')::bigint;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.recommendation_quality_models t WHERE t.version = (k ->> 'version')::bigint;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_search_terms(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.word INTO p FROM public.search_terms t WHERE t.word = (k ->> 'word')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.search_terms t WHERE t.word = (k ->> 'word')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_star_track_tiers(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.sc_track_id INTO p FROM public.star_track_tiers t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.star_track_tiers t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_storage_event_state(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.sc_track_id INTO p FROM public.storage_event_state t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.storage_event_state t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_subscription_snapshot_state(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.singleton INTO p FROM public.subscription_snapshot_state t WHERE t.singleton = (k ->> 'singleton')::boolean;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.subscription_snapshot_state t WHERE t.singleton = (k ->> 'singleton')::boolean;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_sync_queue_action_pauses(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.action_type, t.oauth_app_id INTO p FROM public.sync_queue_action_pauses t WHERE t.oauth_app_id = (k ->> 'oauth_app_id')::uuid AND t.action_type = (k ->> 'action_type')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.sync_queue_action_pauses t WHERE t.oauth_app_id = (k ->> 'oauth_app_id')::uuid AND t.action_type = (k ->> 'action_type')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_taste_model_versions(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.version INTO p FROM public.taste_model_versions t WHERE t.version = (k ->> 'version')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.user_taste_vectors c WHERE c.version = p.version;
    DELETE FROM public.taste_model_versions t WHERE t.version = (k ->> 'version')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_taste_schedule(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.singleton INTO p FROM public.taste_schedule t WHERE t.singleton = (k ->> 'singleton')::boolean;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.taste_schedule t WHERE t.singleton = (k ->> 'singleton')::boolean;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_track_artists(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.artist_id, t.role, t.track_id INTO p FROM public.track_artists t WHERE t.track_id = (k ->> 'track_id')::uuid AND t.artist_id = (k ->> 'artist_id')::uuid AND t.role = (k ->> 'role')::character varying(16);
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.track_artists t WHERE t.track_id = (k ->> 'track_id')::uuid AND t.artist_id = (k ->> 'artist_id')::uuid AND t.role = (k ->> 'role')::character varying(16);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_track_comments(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.id INTO p FROM public.track_comments t WHERE t.id = (k ->> 'id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.track_comments t WHERE t.id = (k ->> 'id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_track_work_aliases(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.alias_key, t.track_id INTO p FROM public.track_work_aliases t WHERE t.track_id = (k ->> 'track_id')::uuid AND t.alias_key = (k ->> 'alias_key')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.track_work_aliases t WHERE t.track_id = (k ->> 'track_id')::uuid AND t.alias_key = (k ->> 'alias_key')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_tracks(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.id, t.sc_track_id INTO p FROM public.tracks t WHERE t.id = (k ->> 'id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.album_tracks c WHERE c.track_id = p.id;
    DELETE FROM public.catalog_work_links c WHERE c.track_id = p.id;
    DELETE FROM public.lyrics_cache c WHERE c.track_id = p.id;
    DELETE FROM public.lyrics_lookup_state c WHERE c.track_id = p.id;
    DELETE FROM public.storage_event_state c WHERE c.sc_track_id = p.sc_track_id;
    DELETE FROM public.track_artist_blocks c WHERE c.track_id = p.id;
    DELETE FROM public.track_artists c WHERE c.track_id = p.id;
    DELETE FROM public.track_work_aliases c WHERE c.track_id = p.id;
    UPDATE public.tracks c SET superseded_by = NULL WHERE c.superseded_by = p.id;
    UPDATE public.wanted_tracks c SET track_id = NULL WHERE c.track_id = p.id;
    DELETE FROM public.tracks t WHERE t.id = (k ->> 'id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_transcription_sync_versions(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.sync_version INTO p FROM public.transcription_sync_versions t WHERE t.sync_version = (k ->> 'sync_version')::character varying(128);
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.transcription_sync_versions t WHERE t.sync_version = (k ->> 'sync_version')::character varying(128);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_transcription_wire_state(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.sc_track_id INTO p FROM public.transcription_wire_state t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.transcription_wire_state t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_user_collection_sync(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.collection, t.user_id INTO p FROM public.user_collection_sync t WHERE t.user_id = (k ->> 'user_id')::text AND t.collection = (k ->> 'collection')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.user_collection_sync t WHERE t.user_id = (k ->> 'user_id')::text AND t.collection = (k ->> 'collection')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_user_followers(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.target_user_urn, t.user_id INTO p FROM public.user_followers t WHERE t.user_id = (k ->> 'user_id')::text AND t.target_user_urn = (k ->> 'target_user_urn')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.user_followers t WHERE t.user_id = (k ->> 'user_id')::text AND t.target_user_urn = (k ->> 'target_user_urn')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_user_owned_playlists(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.playlist_urn, t.user_id INTO p FROM public.user_owned_playlists t WHERE t.user_id = (k ->> 'user_id')::text AND t.playlist_urn = (k ->> 'playlist_urn')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.user_owned_playlists t WHERE t.user_id = (k ->> 'user_id')::text AND t.playlist_urn = (k ->> 'playlist_urn')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_user_owned_tracks(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.sc_track_id, t.user_id INTO p FROM public.user_owned_tracks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.user_owned_tracks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_user_profiles(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.soundcloud_user_id INTO p FROM public.user_profiles t WHERE t.soundcloud_user_id = (k ->> 'soundcloud_user_id')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.user_profiles t WHERE t.soundcloud_user_id = (k ->> 'soundcloud_user_id')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_user_taste_vectors(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.sc_user_id, t.version INTO p FROM public.user_taste_vectors t WHERE t.sc_user_id = (k ->> 'sc_user_id')::text AND t.version = (k ->> 'version')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.user_taste_vectors t WHERE t.sc_user_id = (k ->> 'sc_user_id')::text AND t.version = (k ->> 'version')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_user_web_profiles(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.sc_user_id INTO p FROM public.user_web_profiles t WHERE t.sc_user_id = (k ->> 'sc_user_id')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.user_web_profiles t WHERE t.sc_user_id = (k ->> 'sc_user_id')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_users(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.sc_user_id INTO p FROM public.users t WHERE t.sc_user_id = (k ->> 'sc_user_id')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.users t WHERE t.sc_user_id = (k ->> 'sc_user_id')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_wanted_track_albums(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.album_id, t.wanted_track_id INTO p FROM public.wanted_track_albums t WHERE t.wanted_track_id = (k ->> 'wanted_track_id')::uuid AND t.album_id = (k ->> 'album_id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.wanted_track_albums t WHERE t.wanted_track_id = (k ->> 'wanted_track_id')::uuid AND t.album_id = (k ->> 'album_id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_wanted_track_artists(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.artist_id, t.role, t.wanted_track_id INTO p FROM public.wanted_track_artists t WHERE t.wanted_track_id = (k ->> 'wanted_track_id')::uuid AND t.artist_id = (k ->> 'artist_id')::uuid AND t.role = (k ->> 'role')::character varying(16);
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.wanted_track_artists t WHERE t.wanted_track_id = (k ->> 'wanted_track_id')::uuid AND t.artist_id = (k ->> 'artist_id')::uuid AND t.role = (k ->> 'role')::character varying(16);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_wanted_track_work_aliases(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.alias_key, t.wanted_track_id INTO p FROM public.wanted_track_work_aliases t WHERE t.wanted_track_id = (k ->> 'wanted_track_id')::uuid AND t.alias_key = (k ->> 'alias_key')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.wanted_track_work_aliases t WHERE t.wanted_track_id = (k ->> 'wanted_track_id')::uuid AND t.alias_key = (k ->> 'alias_key')::text;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_wanted_tracks(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.id INTO p FROM public.wanted_tracks t WHERE t.id = (k ->> 'id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.catalog_work_links c WHERE c.wanted_track_id = p.id;
    DELETE FROM public.wanted_track_albums c WHERE c.wanted_track_id = p.id;
    DELETE FROM public.wanted_track_artists c WHERE c.wanted_track_id = p.id;
    DELETE FROM public.wanted_track_work_aliases c WHERE c.wanted_track_id = p.id;
    DELETE FROM public.wanted_tracks t WHERE t.id = (k ->> 'id')::uuid;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge(tbl text, k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    CASE tbl
        WHEN 'admin_maintenance_runs' THEN
            PERFORM public.scd_purge_admin_maintenance_runs(k);
        WHEN 'ai_resolver_budget' THEN
            PERFORM public.scd_purge_ai_resolver_budget(k);
        WHEN 'ai_resolver_cache' THEN
            PERFORM public.scd_purge_ai_resolver_cache(k);
        WHEN 'album_artists' THEN
            PERFORM public.scd_purge_album_artists(k);
        WHEN 'album_tracks' THEN
            PERFORM public.scd_purge_album_tracks(k);
        WHEN 'albums' THEN
            PERFORM public.scd_purge_albums(k);
        WHEN 'artist_attribution_revalidation' THEN
            PERFORM public.scd_purge_artist_attribution_revalidation(k);
        WHEN 'artist_colike' THEN
            PERFORM public.scd_purge_artist_colike(k);
        WHEN 'artist_coplay' THEN
            PERFORM public.scd_purge_artist_coplay(k);
        WHEN 'artist_sc_accounts' THEN
            PERFORM public.scd_purge_artist_sc_accounts(k);
        WHEN 'artist_socials' THEN
            PERFORM public.scd_purge_artist_socials(k);
        WHEN 'artists' THEN
            PERFORM public.scd_purge_artists(k);
        WHEN 'audio_index_wire_state' THEN
            PERFORM public.scd_purge_audio_index_wire_state(k);
        WHEN 'background_job_enqueues' THEN
            PERFORM public.scd_purge_background_job_enqueues(k);
        WHEN 'background_job_failures' THEN
            PERFORM public.scd_purge_background_job_failures(k);
        WHEN 'background_jobs' THEN
            PERFORM public.scd_purge_background_jobs(k);
        WHEN 'background_schedules' THEN
            PERFORM public.scd_purge_background_schedules(k);
        WHEN 'catalog_audience' THEN
            PERFORM public.scd_purge_catalog_audience(k);
        WHEN 'catalog_collection_cursors' THEN
            PERFORM public.scd_purge_catalog_collection_cursors(k);
        WHEN 'catalog_collection_seen' THEN
            PERFORM public.scd_purge_catalog_collection_seen(k);
        WHEN 'catalog_collection_sync' THEN
            PERFORM public.scd_purge_catalog_collection_sync(k);
        WHEN 'catalog_merge_state' THEN
            PERFORM public.scd_purge_catalog_merge_state(k);
        WHEN 'catalog_work_links' THEN
            PERFORM public.scd_purge_catalog_work_links(k);
        WHEN 'cmd_dead' THEN
            PERFORM public.scd_purge_cmd_dead(k);
        WHEN 'discover_tag_counts' THEN
            PERFORM public.scd_purge_discover_tag_counts(k);
        WHEN 'enrich_calibration' THEN
            PERFORM public.scd_purge_enrich_calibration(k);
        WHEN 'job_status' THEN
            PERFORM public.scd_purge_job_status(k);
        WHEN 'lyrics_cache' THEN
            PERFORM public.scd_purge_lyrics_cache(k);
        WHEN 'lyrics_embedding_wire_state' THEN
            PERFORM public.scd_purge_lyrics_embedding_wire_state(k);
        WHEN 'lyrics_lookup_backfill_state' THEN
            PERFORM public.scd_purge_lyrics_lookup_backfill_state(k);
        WHEN 'lyrics_lookup_state' THEN
            PERFORM public.scd_purge_lyrics_lookup_state(k);
        WHEN 'oauth_app_token_issuance_reservations' THEN
            PERFORM public.scd_purge_oauth_app_token_issuance_reservations(k);
        WHEN 'oauth_app_token_refresh_state' THEN
            PERFORM public.scd_purge_oauth_app_token_refresh_state(k);
        WHEN 'oauth_app_tokens' THEN
            PERFORM public.scd_purge_oauth_app_tokens(k);
        WHEN 'pipeline_event_receipts' THEN
            PERFORM public.scd_purge_pipeline_event_receipts(k);
        WHEN 'playlist_legacy_membership_intents' THEN
            PERFORM public.scd_purge_playlist_legacy_membership_intents(k);
        WHEN 'playlist_membership_operations' THEN
            PERFORM public.scd_purge_playlist_membership_operations(k);
        WHEN 'playlist_membership_state' THEN
            PERFORM public.scd_purge_playlist_membership_state(k);
        WHEN 'playlist_reconcile_runs' THEN
            PERFORM public.scd_purge_playlist_reconcile_runs(k);
        WHEN 'playlist_remote_observations' THEN
            PERFORM public.scd_purge_playlist_remote_observations(k);
        WHEN 'playlist_remote_snapshot_tracks' THEN
            PERFORM public.scd_purge_playlist_remote_snapshot_tracks(k);
        WHEN 'playlist_remote_snapshots' THEN
            PERFORM public.scd_purge_playlist_remote_snapshots(k);
        WHEN 'playlist_sweep_cursor' THEN
            PERFORM public.scd_purge_playlist_sweep_cursor(k);
        WHEN 'playlist_track_projection' THEN
            PERFORM public.scd_purge_playlist_track_projection(k);
        WHEN 'playlists' THEN
            PERFORM public.scd_purge_playlists(k);
        WHEN 'recommendation_quality_models' THEN
            PERFORM public.scd_purge_recommendation_quality_models(k);
        WHEN 'search_terms' THEN
            PERFORM public.scd_purge_search_terms(k);
        WHEN 'star_track_tiers' THEN
            PERFORM public.scd_purge_star_track_tiers(k);
        WHEN 'storage_event_state' THEN
            PERFORM public.scd_purge_storage_event_state(k);
        WHEN 'subscription_snapshot_state' THEN
            PERFORM public.scd_purge_subscription_snapshot_state(k);
        WHEN 'sync_queue_action_pauses' THEN
            PERFORM public.scd_purge_sync_queue_action_pauses(k);
        WHEN 'taste_model_versions' THEN
            PERFORM public.scd_purge_taste_model_versions(k);
        WHEN 'taste_schedule' THEN
            PERFORM public.scd_purge_taste_schedule(k);
        WHEN 'track_artists' THEN
            PERFORM public.scd_purge_track_artists(k);
        WHEN 'track_comments' THEN
            PERFORM public.scd_purge_track_comments(k);
        WHEN 'track_work_aliases' THEN
            PERFORM public.scd_purge_track_work_aliases(k);
        WHEN 'tracks' THEN
            PERFORM public.scd_purge_tracks(k);
        WHEN 'transcription_sync_versions' THEN
            PERFORM public.scd_purge_transcription_sync_versions(k);
        WHEN 'transcription_wire_state' THEN
            PERFORM public.scd_purge_transcription_wire_state(k);
        WHEN 'user_collection_sync' THEN
            PERFORM public.scd_purge_user_collection_sync(k);
        WHEN 'user_followers' THEN
            PERFORM public.scd_purge_user_followers(k);
        WHEN 'user_owned_playlists' THEN
            PERFORM public.scd_purge_user_owned_playlists(k);
        WHEN 'user_owned_tracks' THEN
            PERFORM public.scd_purge_user_owned_tracks(k);
        WHEN 'user_profiles' THEN
            PERFORM public.scd_purge_user_profiles(k);
        WHEN 'user_taste_vectors' THEN
            PERFORM public.scd_purge_user_taste_vectors(k);
        WHEN 'user_web_profiles' THEN
            PERFORM public.scd_purge_user_web_profiles(k);
        WHEN 'users' THEN
            PERFORM public.scd_purge_users(k);
        WHEN 'wanted_track_albums' THEN
            PERFORM public.scd_purge_wanted_track_albums(k);
        WHEN 'wanted_track_artists' THEN
            PERFORM public.scd_purge_wanted_track_artists(k);
        WHEN 'wanted_track_work_aliases' THEN
            PERFORM public.scd_purge_wanted_track_work_aliases(k);
        WHEN 'wanted_tracks' THEN
            PERFORM public.scd_purge_wanted_tracks(k);
        ELSE
            RAISE EXCEPTION 'public.scd_purge: unknown table %', tbl;
    END CASE;
    RETURN;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_intent_soundcloud_connections(k jsonb)
RETURNS boolean
LANGUAGE plpgsql
AS $f$
DECLARE
    v bigint;
    key jsonb;
BEGIN
    IF public.scd_role() <> 'core' THEN
        RAISE EXCEPTION 'scd_purge_intent: only core purges public.soundcloud_connections' USING ERRCODE = 'insufficient_privilege';
    END IF;
    SELECT t.token_v, jsonb_build_object('id', t.id) INTO v, key FROM public.soundcloud_connections t WHERE t.id = (k ->> 'id')::uuid FOR UPDATE;
    IF NOT FOUND THEN
        RETURN false;
    END IF;
    INSERT INTO public.repl_purges (tbl, pk, purged_v) VALUES ('soundcloud_connections', key, v);
    DELETE FROM public.soundcloud_connections t WHERE t.id = (k ->> 'id')::uuid;
    RETURN true;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_intent_sync_queue(k jsonb)
RETURNS boolean
LANGUAGE plpgsql
AS $f$
DECLARE
    v bigint;
    key jsonb;
BEGIN
    IF public.scd_role() <> 'core' THEN
        RAISE EXCEPTION 'scd_purge_intent: only core purges public.sync_queue' USING ERRCODE = 'insufficient_privilege';
    END IF;
    SELECT t.intent_v, jsonb_build_object('id', t.id) INTO v, key FROM public.sync_queue t WHERE t.id = (k ->> 'id')::uuid FOR UPDATE;
    IF NOT FOUND THEN
        RETURN false;
    END IF;
    INSERT INTO public.repl_purges (tbl, pk, purged_v) VALUES ('sync_queue', key, v);
    DELETE FROM public.sync_queue t WHERE t.id = (k ->> 'id')::uuid;
    RETURN true;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_intent_user_followings(k jsonb)
RETURNS boolean
LANGUAGE plpgsql
AS $f$
DECLARE
    v bigint;
    key jsonb;
BEGIN
    IF public.scd_role() <> 'core' THEN
        RAISE EXCEPTION 'scd_purge_intent: only core purges public.user_followings' USING ERRCODE = 'insufficient_privilege';
    END IF;
    SELECT t.wanted_v, jsonb_build_object('user_id', t.user_id, 'target_user_urn', t.target_user_urn) INTO v, key FROM public.user_followings t WHERE t.user_id = (k ->> 'user_id')::text AND t.target_user_urn = (k ->> 'target_user_urn')::text FOR UPDATE;
    IF NOT FOUND THEN
        RETURN false;
    END IF;
    INSERT INTO public.repl_purges (tbl, pk, purged_v) VALUES ('user_followings', key, v);
    DELETE FROM public.user_followings t WHERE t.user_id = (k ->> 'user_id')::text AND t.target_user_urn = (k ->> 'target_user_urn')::text;
    RETURN true;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_intent_user_likes_playlists(k jsonb)
RETURNS boolean
LANGUAGE plpgsql
AS $f$
DECLARE
    v bigint;
    key jsonb;
BEGIN
    IF public.scd_role() <> 'core' THEN
        RAISE EXCEPTION 'scd_purge_intent: only core purges public.user_likes_playlists' USING ERRCODE = 'insufficient_privilege';
    END IF;
    SELECT t.wanted_v, jsonb_build_object('user_id', t.user_id, 'playlist_urn', t.playlist_urn) INTO v, key FROM public.user_likes_playlists t WHERE t.user_id = (k ->> 'user_id')::text AND t.playlist_urn = (k ->> 'playlist_urn')::text FOR UPDATE;
    IF NOT FOUND THEN
        RETURN false;
    END IF;
    INSERT INTO public.repl_purges (tbl, pk, purged_v) VALUES ('user_likes_playlists', key, v);
    DELETE FROM public.user_likes_playlists t WHERE t.user_id = (k ->> 'user_id')::text AND t.playlist_urn = (k ->> 'playlist_urn')::text;
    RETURN true;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_intent_user_likes_tracks(k jsonb)
RETURNS boolean
LANGUAGE plpgsql
AS $f$
DECLARE
    v bigint;
    key jsonb;
BEGIN
    IF public.scd_role() <> 'core' THEN
        RAISE EXCEPTION 'scd_purge_intent: only core purges public.user_likes_tracks' USING ERRCODE = 'insufficient_privilege';
    END IF;
    SELECT t.wanted_v, jsonb_build_object('user_id', t.user_id, 'sc_track_id', t.sc_track_id) INTO v, key FROM public.user_likes_tracks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text FOR UPDATE;
    IF NOT FOUND THEN
        RETURN false;
    END IF;
    INSERT INTO public.repl_purges (tbl, pk, purged_v) VALUES ('user_likes_tracks', key, v);
    DELETE FROM public.user_likes_tracks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text;
    RETURN true;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_purge_intent(tbl text, k jsonb)
RETURNS boolean
LANGUAGE plpgsql
AS $f$
BEGIN
    CASE tbl
        WHEN 'soundcloud_connections' THEN
            RETURN public.scd_purge_intent_soundcloud_connections(k);
        WHEN 'sync_queue' THEN
            RETURN public.scd_purge_intent_sync_queue(k);
        WHEN 'user_followings' THEN
            RETURN public.scd_purge_intent_user_followings(k);
        WHEN 'user_likes_playlists' THEN
            RETURN public.scd_purge_intent_user_likes_playlists(k);
        WHEN 'user_likes_tracks' THEN
            RETURN public.scd_purge_intent_user_likes_tracks(k);
        ELSE
            RAISE EXCEPTION 'public.scd_purge_intent: unknown table %', tbl;
    END CASE;
END
$f$;

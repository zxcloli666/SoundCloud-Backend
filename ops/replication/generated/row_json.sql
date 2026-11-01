CREATE OR REPLACE FUNCTION public.scd_row_json_admin_catalog_requests(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.admin_catalog_requests t WHERE t.request_id = (k ->> 'request_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_admin_maintenance_runs(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.admin_maintenance_runs t WHERE t.kind = (k ->> 'kind')::character varying(64)
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_ai_resolver_budget(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.ai_resolver_budget t WHERE t.spent_on = (k ->> 'spent_on')::date
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_ai_resolver_cache(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.ai_resolver_cache t WHERE t.cache_key = (k ->> 'cache_key')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_album_artists(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.album_artists t WHERE t.album_id = (k ->> 'album_id')::uuid AND t.artist_id = (k ->> 'artist_id')::uuid AND t.role = (k ->> 'role')::character varying(16)
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_album_tracks(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.album_tracks t WHERE t.album_id = (k ->> 'album_id')::uuid AND t.track_id = (k ->> 'track_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_albums(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.albums t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_artist_attribution_revalidation(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.artist_attribution_revalidation t WHERE t.singleton = (k ->> 'singleton')::boolean
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_artist_colike(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.artist_colike t WHERE t.a_id = (k ->> 'a_id')::uuid AND t.b_id = (k ->> 'b_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_artist_coplay(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.artist_coplay t WHERE t.a_id = (k ->> 'a_id')::uuid AND t.b_id = (k ->> 'b_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_artist_sc_accounts(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.artist_sc_accounts t WHERE t.artist_id = (k ->> 'artist_id')::uuid AND t.sc_user_id = (k ->> 'sc_user_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_artist_socials(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.artist_socials t WHERE t.artist_id = (k ->> 'artist_id')::uuid AND t.url = (k ->> 'url')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_artists(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.artists t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_audio_index_wire_state(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.audio_index_wire_state t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_background_job_enqueues(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.background_job_enqueues t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_background_job_failures(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.background_job_failures t WHERE t.id = (k ->> 'id')::uuid AND t.generation = (k ->> 'generation')::bigint
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_background_jobs(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.background_jobs t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_background_schedules(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.background_schedules t WHERE t.kind = (k ->> 'kind')::character varying(96)
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_catalog_audience(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.catalog_audience t WHERE t.subject_urn = (k ->> 'subject_urn')::text AND t.relation = (k ->> 'relation')::text AND t.user_urn = (k ->> 'user_urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_catalog_collection_cursors(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.catalog_collection_cursors t WHERE t.subject_id = (k ->> 'subject_id')::text AND t.collection = (k ->> 'collection')::text AND t.scope = (k ->> 'scope')::text AND t.cursor = (k ->> 'cursor')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_catalog_collection_seen(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.catalog_collection_seen t WHERE t.subject_id = (k ->> 'subject_id')::text AND t.collection = (k ->> 'collection')::text AND t.scope = (k ->> 'scope')::text AND t.entity_key = (k ->> 'entity_key')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_catalog_collection_sync(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.catalog_collection_sync t WHERE t.subject_id = (k ->> 'subject_id')::text AND t.collection = (k ->> 'collection')::text AND t.scope = (k ->> 'scope')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_catalog_merge_state(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.catalog_merge_state t WHERE t.singleton = (k ->> 'singleton')::boolean
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_catalog_work_links(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.catalog_work_links t WHERE t.wanted_track_id = (k ->> 'wanted_track_id')::uuid AND t.track_id = (k ->> 'track_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_cdn_tracks(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) - ARRAY['last_accessed_at']::text[] FROM public.cdn_tracks t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_cmd_dead(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.cmd_dead t WHERE t.node = (k ->> 'node')::text AND t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_diag_bundles(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.diag_bundles t WHERE t.code = (k ->> 'code')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_diag_events(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.diag_events t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_diag_issue_days(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.diag_issue_days t WHERE t.fingerprint = (k ->> 'fingerprint')::text AND t.day = (k ->> 'day')::date AND t.host = (k ->> 'host')::text AND t.app_version = (k ->> 'app_version')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_diag_issue_origins(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.diag_issue_origins t WHERE t.fingerprint = (k ->> 'fingerprint')::text AND t.day = (k ->> 'day')::date AND t.host = (k ->> 'host')::text AND t.origin = (k ->> 'origin')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_discover_promoted(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.discover_promoted t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_discover_settings(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.discover_settings t WHERE t.id = (k ->> 'id')::smallint
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_discover_tag_counts(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.discover_tag_counts t WHERE t.tag = (k ->> 'tag')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_disliked_tracks(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.disliked_tracks t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_enrich_calibration(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.enrich_calibration t WHERE t.source = (k ->> 'source')::character varying(16) AND t.raw_bin_low = (k ->> 'raw_bin_low')::real AND t.raw_bin_high = (k ->> 'raw_bin_high')::real
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_entity_reads(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.entity_reads t WHERE t.entity_urn = (k ->> 'entity_urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_featured_items(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.featured_items t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_job_status(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.job_status t WHERE t.kind = (k ->> 'kind')::text AND t.dedup_key = (k ->> 'dedup_key')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_link_requests(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.link_requests t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_listening_history(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.listening_history t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_login_requests(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.login_requests t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_lyrics_cache(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.lyrics_cache t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_lyrics_embedding_wire_state(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.lyrics_embedding_wire_state t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_lyrics_lookup_backfill_state(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.lyrics_lookup_backfill_state t WHERE t.singleton = (k ->> 'singleton')::boolean
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_lyrics_lookup_state(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.lyrics_lookup_state t WHERE t.track_id = (k ->> 'track_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_oauth_app_request_cooldowns(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.oauth_app_request_cooldowns t WHERE t.oauth_app_id = (k ->> 'oauth_app_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_oauth_app_token_issuance_reservations(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.oauth_app_token_issuance_reservations t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_oauth_app_token_refresh_state(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.oauth_app_token_refresh_state t WHERE t.oauth_app_id = (k ->> 'oauth_app_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_oauth_app_tokens(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.oauth_app_tokens t WHERE t.oauth_app_id = (k ->> 'oauth_app_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_oauth_apps(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) - ARRAY['last_used_at']::text[] FROM public.oauth_apps t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_pipeline_event_receipts(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.pipeline_event_receipts t WHERE t.consumer = (k ->> 'consumer')::character varying(96) AND t.stream = (k ->> 'stream')::character varying(128) AND t.stream_sequence = (k ->> 'stream_sequence')::bigint AND t.event_published_at = (k ->> 'event_published_at')::timestamp with time zone
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_playlist_legacy_membership_intents(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.playlist_legacy_membership_intents t WHERE t.archive_id = (k ->> 'archive_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_playlist_membership_operations(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.playlist_membership_operations t WHERE t.operation_id = (k ->> 'operation_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_playlist_membership_state(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.playlist_membership_state t WHERE t.playlist_urn = (k ->> 'playlist_urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_playlist_reconcile_runs(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.playlist_reconcile_runs t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_playlist_remote_observations(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) - ARRAY['write_eligible']::text[] FROM public.playlist_remote_observations t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_playlist_remote_snapshot_tracks(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.playlist_remote_snapshot_tracks t WHERE t.snapshot_id = (k ->> 'snapshot_id')::uuid AND t."position" = (k ->> 'position')::integer
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_playlist_remote_snapshots(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.playlist_remote_snapshots t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_playlist_sweep_cursor(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.playlist_sweep_cursor t WHERE t.id = (k ->> 'id')::boolean
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_playlist_track_projection(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.playlist_track_projection t WHERE t.playlist_urn = (k ->> 'playlist_urn')::text AND t."position" = (k ->> 'position')::integer
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_playlists(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.playlists t WHERE t.urn = (k ->> 'urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_recommendation_quality_models(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.recommendation_quality_models t WHERE t.version = (k ->> 'version')::bigint
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_repl_checksums(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.repl_checksums t WHERE t.node = (k ->> 'node')::text AND t.tbl = (k ->> 'tbl')::text AND t.range = (k ->> 'range')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_repl_tombstones(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.repl_tombstones t WHERE t.tbl = (k ->> 'tbl')::text AND t.pk = (k ->> 'pk')::jsonb AND t.v = (k ->> 'v')::bigint
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_sc_egress_health(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.sc_egress_health t WHERE t.channel = (k ->> 'channel')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_sc_track_counters(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.sc_track_counters t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_search_terms(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.search_terms t WHERE t.word = (k ->> 'word')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_sessions(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.sessions t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_soundcloud_connections(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.soundcloud_connections t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_star_track_tiers(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.star_track_tiers t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_storage_event_repairs(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.storage_event_repairs t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_storage_event_state(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.storage_event_state t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_subscription_snapshot_state(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.subscription_snapshot_state t WHERE t.singleton = (k ->> 'singleton')::boolean
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_subscriptions(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.subscriptions t WHERE t.user_urn = (k ->> 'user_urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_sync_queue(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.sync_queue t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_sync_queue_action_pauses(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.sync_queue_action_pauses t WHERE t.oauth_app_id = (k ->> 'oauth_app_id')::uuid AND t.action_type = (k ->> 'action_type')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_taste_model_versions(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.taste_model_versions t WHERE t.version = (k ->> 'version')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_taste_schedule(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.taste_schedule t WHERE t.singleton = (k ->> 'singleton')::boolean
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_track_artist_blocks(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.track_artist_blocks t WHERE t.track_id = (k ->> 'track_id')::uuid AND t.artist_id = (k ->> 'artist_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_track_artists(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.track_artists t WHERE t.track_id = (k ->> 'track_id')::uuid AND t.artist_id = (k ->> 'artist_id')::uuid AND t.role = (k ->> 'role')::character varying(16)
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_track_comments(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.track_comments t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_track_work_aliases(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.track_work_aliases t WHERE t.track_id = (k ->> 'track_id')::uuid AND t.alias_key = (k ->> 'alias_key')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_tracks(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) - ARRAY['last_read_at']::text[] FROM public.tracks t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_transcription_sync_versions(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.transcription_sync_versions t WHERE t.sync_version = (k ->> 'sync_version')::character varying(128)
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_transcription_wire_state(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.transcription_wire_state t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_user_auras(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.user_auras t WHERE t.user_urn = (k ->> 'user_urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_user_blocked_artists(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.user_blocked_artists t WHERE t.sc_user_id = (k ->> 'sc_user_id')::text AND t.kind = (k ->> 'kind')::text AND t.target_id = (k ->> 'target_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_user_collection_sync(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.user_collection_sync t WHERE t.user_id = (k ->> 'user_id')::text AND t.collection = (k ->> 'collection')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_user_events(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.user_events t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_user_followers(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) - ARRAY['last_read_at']::text[] FROM public.user_followers t WHERE t.user_id = (k ->> 'user_id')::text AND t.target_user_urn = (k ->> 'target_user_urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_user_followings(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) - ARRAY['last_read_at']::text[] FROM public.user_followings t WHERE t.user_id = (k ->> 'user_id')::text AND t.target_user_urn = (k ->> 'target_user_urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_user_like_ranks(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.user_like_ranks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_user_likes_playlists(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) - ARRAY['last_read_at']::text[] FROM public.user_likes_playlists t WHERE t.user_id = (k ->> 'user_id')::text AND t.playlist_urn = (k ->> 'playlist_urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_user_likes_tracks(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) - ARRAY['last_read_at']::text[] FROM public.user_likes_tracks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_user_owned_playlists(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) - ARRAY['last_read_at']::text[] FROM public.user_owned_playlists t WHERE t.user_id = (k ->> 'user_id')::text AND t.playlist_urn = (k ->> 'playlist_urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_user_owned_tracks(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) - ARRAY['last_read_at']::text[] FROM public.user_owned_tracks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_user_profiles(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.user_profiles t WHERE t.soundcloud_user_id = (k ->> 'soundcloud_user_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_user_superlike_ranks(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.user_superlike_ranks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_user_superlikes(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.user_superlikes t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_user_taste_vectors(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.user_taste_vectors t WHERE t.sc_user_id = (k ->> 'sc_user_id')::text AND t.version = (k ->> 'version')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_user_web_profiles(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.user_web_profiles t WHERE t.sc_user_id = (k ->> 'sc_user_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_users(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) - ARRAY['last_read_at']::text[] FROM public.users t WHERE t.sc_user_id = (k ->> 'sc_user_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_wanted_track_albums(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.wanted_track_albums t WHERE t.wanted_track_id = (k ->> 'wanted_track_id')::uuid AND t.album_id = (k ->> 'album_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_wanted_track_artists(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.wanted_track_artists t WHERE t.wanted_track_id = (k ->> 'wanted_track_id')::uuid AND t.artist_id = (k ->> 'artist_id')::uuid AND t.role = (k ->> 'role')::character varying(16)
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_wanted_track_work_aliases(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.wanted_track_work_aliases t WHERE t.wanted_track_id = (k ->> 'wanted_track_id')::uuid AND t.alias_key = (k ->> 'alias_key')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json_wanted_tracks(k jsonb)
RETURNS jsonb
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT pg_catalog.to_jsonb(t) FROM public.wanted_tracks t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_json(tbl text, k jsonb)
RETURNS jsonb
LANGUAGE plpgsql STABLE
AS $f$
BEGIN
    CASE tbl
        WHEN 'admin_catalog_requests' THEN
            RETURN public.scd_row_json_admin_catalog_requests(k);
        WHEN 'admin_maintenance_runs' THEN
            RETURN public.scd_row_json_admin_maintenance_runs(k);
        WHEN 'ai_resolver_budget' THEN
            RETURN public.scd_row_json_ai_resolver_budget(k);
        WHEN 'ai_resolver_cache' THEN
            RETURN public.scd_row_json_ai_resolver_cache(k);
        WHEN 'album_artists' THEN
            RETURN public.scd_row_json_album_artists(k);
        WHEN 'album_tracks' THEN
            RETURN public.scd_row_json_album_tracks(k);
        WHEN 'albums' THEN
            RETURN public.scd_row_json_albums(k);
        WHEN 'artist_attribution_revalidation' THEN
            RETURN public.scd_row_json_artist_attribution_revalidation(k);
        WHEN 'artist_colike' THEN
            RETURN public.scd_row_json_artist_colike(k);
        WHEN 'artist_coplay' THEN
            RETURN public.scd_row_json_artist_coplay(k);
        WHEN 'artist_sc_accounts' THEN
            RETURN public.scd_row_json_artist_sc_accounts(k);
        WHEN 'artist_socials' THEN
            RETURN public.scd_row_json_artist_socials(k);
        WHEN 'artists' THEN
            RETURN public.scd_row_json_artists(k);
        WHEN 'audio_index_wire_state' THEN
            RETURN public.scd_row_json_audio_index_wire_state(k);
        WHEN 'background_job_enqueues' THEN
            RETURN public.scd_row_json_background_job_enqueues(k);
        WHEN 'background_job_failures' THEN
            RETURN public.scd_row_json_background_job_failures(k);
        WHEN 'background_jobs' THEN
            RETURN public.scd_row_json_background_jobs(k);
        WHEN 'background_schedules' THEN
            RETURN public.scd_row_json_background_schedules(k);
        WHEN 'catalog_audience' THEN
            RETURN public.scd_row_json_catalog_audience(k);
        WHEN 'catalog_collection_cursors' THEN
            RETURN public.scd_row_json_catalog_collection_cursors(k);
        WHEN 'catalog_collection_seen' THEN
            RETURN public.scd_row_json_catalog_collection_seen(k);
        WHEN 'catalog_collection_sync' THEN
            RETURN public.scd_row_json_catalog_collection_sync(k);
        WHEN 'catalog_merge_state' THEN
            RETURN public.scd_row_json_catalog_merge_state(k);
        WHEN 'catalog_work_links' THEN
            RETURN public.scd_row_json_catalog_work_links(k);
        WHEN 'cdn_tracks' THEN
            RETURN public.scd_row_json_cdn_tracks(k);
        WHEN 'cmd_dead' THEN
            RETURN public.scd_row_json_cmd_dead(k);
        WHEN 'diag_bundles' THEN
            RETURN public.scd_row_json_diag_bundles(k);
        WHEN 'diag_events' THEN
            RETURN public.scd_row_json_diag_events(k);
        WHEN 'diag_issue_days' THEN
            RETURN public.scd_row_json_diag_issue_days(k);
        WHEN 'diag_issue_origins' THEN
            RETURN public.scd_row_json_diag_issue_origins(k);
        WHEN 'discover_promoted' THEN
            RETURN public.scd_row_json_discover_promoted(k);
        WHEN 'discover_settings' THEN
            RETURN public.scd_row_json_discover_settings(k);
        WHEN 'discover_tag_counts' THEN
            RETURN public.scd_row_json_discover_tag_counts(k);
        WHEN 'disliked_tracks' THEN
            RETURN public.scd_row_json_disliked_tracks(k);
        WHEN 'enrich_calibration' THEN
            RETURN public.scd_row_json_enrich_calibration(k);
        WHEN 'entity_reads' THEN
            RETURN public.scd_row_json_entity_reads(k);
        WHEN 'featured_items' THEN
            RETURN public.scd_row_json_featured_items(k);
        WHEN 'job_status' THEN
            RETURN public.scd_row_json_job_status(k);
        WHEN 'link_requests' THEN
            RETURN public.scd_row_json_link_requests(k);
        WHEN 'listening_history' THEN
            RETURN public.scd_row_json_listening_history(k);
        WHEN 'login_requests' THEN
            RETURN public.scd_row_json_login_requests(k);
        WHEN 'lyrics_cache' THEN
            RETURN public.scd_row_json_lyrics_cache(k);
        WHEN 'lyrics_embedding_wire_state' THEN
            RETURN public.scd_row_json_lyrics_embedding_wire_state(k);
        WHEN 'lyrics_lookup_backfill_state' THEN
            RETURN public.scd_row_json_lyrics_lookup_backfill_state(k);
        WHEN 'lyrics_lookup_state' THEN
            RETURN public.scd_row_json_lyrics_lookup_state(k);
        WHEN 'oauth_app_request_cooldowns' THEN
            RETURN public.scd_row_json_oauth_app_request_cooldowns(k);
        WHEN 'oauth_app_token_issuance_reservations' THEN
            RETURN public.scd_row_json_oauth_app_token_issuance_reservations(k);
        WHEN 'oauth_app_token_refresh_state' THEN
            RETURN public.scd_row_json_oauth_app_token_refresh_state(k);
        WHEN 'oauth_app_tokens' THEN
            RETURN public.scd_row_json_oauth_app_tokens(k);
        WHEN 'oauth_apps' THEN
            RETURN public.scd_row_json_oauth_apps(k);
        WHEN 'pipeline_event_receipts' THEN
            RETURN public.scd_row_json_pipeline_event_receipts(k);
        WHEN 'playlist_legacy_membership_intents' THEN
            RETURN public.scd_row_json_playlist_legacy_membership_intents(k);
        WHEN 'playlist_membership_operations' THEN
            RETURN public.scd_row_json_playlist_membership_operations(k);
        WHEN 'playlist_membership_state' THEN
            RETURN public.scd_row_json_playlist_membership_state(k);
        WHEN 'playlist_reconcile_runs' THEN
            RETURN public.scd_row_json_playlist_reconcile_runs(k);
        WHEN 'playlist_remote_observations' THEN
            RETURN public.scd_row_json_playlist_remote_observations(k);
        WHEN 'playlist_remote_snapshot_tracks' THEN
            RETURN public.scd_row_json_playlist_remote_snapshot_tracks(k);
        WHEN 'playlist_remote_snapshots' THEN
            RETURN public.scd_row_json_playlist_remote_snapshots(k);
        WHEN 'playlist_sweep_cursor' THEN
            RETURN public.scd_row_json_playlist_sweep_cursor(k);
        WHEN 'playlist_track_projection' THEN
            RETURN public.scd_row_json_playlist_track_projection(k);
        WHEN 'playlists' THEN
            RETURN public.scd_row_json_playlists(k);
        WHEN 'recommendation_quality_models' THEN
            RETURN public.scd_row_json_recommendation_quality_models(k);
        WHEN 'repl_checksums' THEN
            RETURN public.scd_row_json_repl_checksums(k);
        WHEN 'repl_tombstones' THEN
            RETURN public.scd_row_json_repl_tombstones(k);
        WHEN 'sc_egress_health' THEN
            RETURN public.scd_row_json_sc_egress_health(k);
        WHEN 'sc_track_counters' THEN
            RETURN public.scd_row_json_sc_track_counters(k);
        WHEN 'search_terms' THEN
            RETURN public.scd_row_json_search_terms(k);
        WHEN 'sessions' THEN
            RETURN public.scd_row_json_sessions(k);
        WHEN 'soundcloud_connections' THEN
            RETURN public.scd_row_json_soundcloud_connections(k);
        WHEN 'star_track_tiers' THEN
            RETURN public.scd_row_json_star_track_tiers(k);
        WHEN 'storage_event_repairs' THEN
            RETURN public.scd_row_json_storage_event_repairs(k);
        WHEN 'storage_event_state' THEN
            RETURN public.scd_row_json_storage_event_state(k);
        WHEN 'subscription_snapshot_state' THEN
            RETURN public.scd_row_json_subscription_snapshot_state(k);
        WHEN 'subscriptions' THEN
            RETURN public.scd_row_json_subscriptions(k);
        WHEN 'sync_queue' THEN
            RETURN public.scd_row_json_sync_queue(k);
        WHEN 'sync_queue_action_pauses' THEN
            RETURN public.scd_row_json_sync_queue_action_pauses(k);
        WHEN 'taste_model_versions' THEN
            RETURN public.scd_row_json_taste_model_versions(k);
        WHEN 'taste_schedule' THEN
            RETURN public.scd_row_json_taste_schedule(k);
        WHEN 'track_artist_blocks' THEN
            RETURN public.scd_row_json_track_artist_blocks(k);
        WHEN 'track_artists' THEN
            RETURN public.scd_row_json_track_artists(k);
        WHEN 'track_comments' THEN
            RETURN public.scd_row_json_track_comments(k);
        WHEN 'track_work_aliases' THEN
            RETURN public.scd_row_json_track_work_aliases(k);
        WHEN 'tracks' THEN
            RETURN public.scd_row_json_tracks(k);
        WHEN 'transcription_sync_versions' THEN
            RETURN public.scd_row_json_transcription_sync_versions(k);
        WHEN 'transcription_wire_state' THEN
            RETURN public.scd_row_json_transcription_wire_state(k);
        WHEN 'user_auras' THEN
            RETURN public.scd_row_json_user_auras(k);
        WHEN 'user_blocked_artists' THEN
            RETURN public.scd_row_json_user_blocked_artists(k);
        WHEN 'user_collection_sync' THEN
            RETURN public.scd_row_json_user_collection_sync(k);
        WHEN 'user_events' THEN
            RETURN public.scd_row_json_user_events(k);
        WHEN 'user_followers' THEN
            RETURN public.scd_row_json_user_followers(k);
        WHEN 'user_followings' THEN
            RETURN public.scd_row_json_user_followings(k);
        WHEN 'user_like_ranks' THEN
            RETURN public.scd_row_json_user_like_ranks(k);
        WHEN 'user_likes_playlists' THEN
            RETURN public.scd_row_json_user_likes_playlists(k);
        WHEN 'user_likes_tracks' THEN
            RETURN public.scd_row_json_user_likes_tracks(k);
        WHEN 'user_owned_playlists' THEN
            RETURN public.scd_row_json_user_owned_playlists(k);
        WHEN 'user_owned_tracks' THEN
            RETURN public.scd_row_json_user_owned_tracks(k);
        WHEN 'user_profiles' THEN
            RETURN public.scd_row_json_user_profiles(k);
        WHEN 'user_superlike_ranks' THEN
            RETURN public.scd_row_json_user_superlike_ranks(k);
        WHEN 'user_superlikes' THEN
            RETURN public.scd_row_json_user_superlikes(k);
        WHEN 'user_taste_vectors' THEN
            RETURN public.scd_row_json_user_taste_vectors(k);
        WHEN 'user_web_profiles' THEN
            RETURN public.scd_row_json_user_web_profiles(k);
        WHEN 'users' THEN
            RETURN public.scd_row_json_users(k);
        WHEN 'wanted_track_albums' THEN
            RETURN public.scd_row_json_wanted_track_albums(k);
        WHEN 'wanted_track_artists' THEN
            RETURN public.scd_row_json_wanted_track_artists(k);
        WHEN 'wanted_track_work_aliases' THEN
            RETURN public.scd_row_json_wanted_track_work_aliases(k);
        WHEN 'wanted_tracks' THEN
            RETURN public.scd_row_json_wanted_tracks(k);
        ELSE
            RAISE EXCEPTION 'public.scd_row_json: unknown table %', tbl;
    END CASE;
END
$f$;

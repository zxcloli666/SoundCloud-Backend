CREATE OR REPLACE FUNCTION public.scd_row_hash_admin_maintenance_runs(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.changed, t.completed_at, t.cursor_text, t.cursor_uuid, t.kind, t.merged, t.phase, t.run_id, t.scanned, t.skipped, t.started_at, t.status, t.updated_at)::text) FROM public.admin_maintenance_runs t WHERE t.kind = (k ->> 'kind')::character varying(64)
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_ai_resolver_budget(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.spent, t.spent_on)::text) FROM public.ai_resolver_budget t WHERE t.spent_on = (k ->> 'spent_on')::date
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_ai_resolver_cache(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.cache_key, t.expires_at, t.reply)::text) FROM public.ai_resolver_cache t WHERE t.cache_key = (k ->> 'cache_key')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_album_artists(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.album_id, t.artist_id, t.role)::text) FROM public.album_artists t WHERE t.album_id = (k ->> 'album_id')::uuid AND t.artist_id = (k ->> 'artist_id')::uuid AND t.role = (k ->> 'role')::character varying(16)
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_album_tracks(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.album_id, t.disc_number, t."position", t.track_id)::text) FROM public.album_tracks t WHERE t.album_id = (k ->> 'album_id')::uuid AND t.track_id = (k ->> 'track_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_albums(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.aggregates_updated_at, t.confidence, t.cover_url, t.created_at, t.genius_album_id, t.id, t.is_star_artist, t.mb_release_id, t.normalized_title, t.popularity_score, t.primary_artist_id, t.release_date, t.release_year, t.source, t.spotify_album_id, t.title, t.total_duration_ms, t.track_count, t.type, t.updated_at)::text) FROM public.albums t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_artist_attribution_revalidation(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.credits_removed, t.identity_completed_at, t.identity_cursor_account, t.identity_cursor_artist, t.singleton, t.tracks_reset, t.updated_at, t.walker_completed_at)::text) FROM public.artist_attribution_revalidation t WHERE t.singleton = (k ->> 'singleton')::boolean
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_artist_colike(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.a_id, t.b_id, t.co, t.updated_at, t.w)::text) FROM public.artist_colike t WHERE t.a_id = (k ->> 'a_id')::uuid AND t.b_id = (k ->> 'b_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_artist_coplay(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.a_id, t.b_id, t.last_seen, t.weight)::text) FROM public.artist_coplay t WHERE t.a_id = (k ->> 'a_id')::uuid AND t.b_id = (k ->> 'b_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_artist_sc_accounts(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.artist_id, t.created_at, t.notes, t.role, t.sc_user_id, t.source, t.updated_at, t.verified)::text) FROM public.artist_sc_accounts t WHERE t.artist_id = (k ->> 'artist_id')::uuid AND t.sc_user_id = (k ->> 'sc_user_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_artist_socials(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.artist_id, t.created_at, t.kind, t.source, t.updated_at, t.url, t.verified)::text) FROM public.artist_socials t WHERE t.artist_id = (k ->> 'artist_id')::uuid AND t.url = (k ->> 'url')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_artists(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.account_walk_locked_at, t.aggregates_updated_at, t.album_count_denorm, t.avatar_url, t.bio, t.confidence, t.country, t.crawl_attempts, t.crawl_dead, t.crawl_fail_count, t.created_at, t.genius_artist_id, t.genius_crawl_offset, t.genius_crawled_at, t.genius_locked_at, t.genius_next_run_at, t.id, t.interest_score, t.is_star, t.isni, t.last_account_walk_at, t.last_crawled_at, t.mb_artist_id, t.mb_crawl_offset, t.mb_crawled_at, t.mb_locked_at, t.mb_next_run_at, t.merged_into, t.monthly_listeners, t.name, t.normalized_name, t.popularity_score, t.sc_user_id, t.source, t.spotify_artist_id, t.star_aura_id, t.star_custom_hex, t.tags, t.track_count_featured, t.track_count_primary, t.trending_score, t.updated_at)::text) FROM public.artists t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_audio_index_wire_state(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.attempt, t.completed_at, t.dispatched_at, t.outcome_rank, t.outcome_reason, t.outcome_status, t.quarantine_reason, t.result_consumer, t.result_lease_expires_at, t.result_lease_id, t.result_published_at, t.result_stream, t.result_stream_sequence, t.sc_track_id, t.status, t.updated_at, t.upload_generation)::text) FROM public.audio_index_wire_state t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_background_job_enqueues(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.accepted_at, t.id)::text) FROM public.background_job_enqueues t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_background_job_failures(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.attempts, t.created_at, t.dedup_key, t.failed_at, t.generation, t.id, t.kind, t.lane, t.last_error, t.max_attempts, t.payload, t.priority)::text) FROM public.background_job_failures t WHERE t.id = (k ->> 'id')::uuid AND t.generation = (k ->> 'generation')::bigint
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_background_jobs(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.attempts, t.available_at, t.created_at, t.dedup_key, t.generation, t.id, t.kind, t.lane, t.last_error, t.lease_expires_at, t.lease_generation, t.lease_id, t.leased_by, t.max_attempts, t.payload, t.priority, t.updated_at)::text) FROM public.background_jobs t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_background_schedules(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.enabled, t.interval_seconds, t.kind, t.lane, t.max_attempts, t.next_run_at, t.priority, t.updated_at)::text) FROM public.background_schedules t WHERE t.kind = (k ->> 'kind')::character varying(96)
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_catalog_audience(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.created_at, t.relation, t.subject_urn, t.synced_at, t.user_urn)::text) FROM public.catalog_audience t WHERE t.subject_urn = (k ->> 'subject_urn')::text AND t.relation = (k ->> 'relation')::text AND t.user_urn = (k ->> 'user_urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_catalog_collection_cursors(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.collection, t.cursor, t.scope, t.subject_id)::text) FROM public.catalog_collection_cursors t WHERE t.subject_id = (k ->> 'subject_id')::text AND t.collection = (k ->> 'collection')::text AND t.scope = (k ->> 'scope')::text AND t.cursor = (k ->> 'cursor')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_catalog_collection_seen(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.collection, t.entity_key, t.scope, t.snapshot_id, t.subject_id)::text) FROM public.catalog_collection_seen t WHERE t.subject_id = (k ->> 'subject_id')::text AND t.collection = (k ->> 'collection')::text AND t.scope = (k ->> 'scope')::text AND t.entity_key = (k ->> 'entity_key')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_catalog_collection_sync(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.collection, t.complete, t.generation, t.item_count, t.job_id, t.next_cursor, t.page_count, t.scope, t.snapshot_id, t.started_at, t.subject_id, t.synced_at, t.updated_at, t.verified_at)::text) FROM public.catalog_collection_sync t WHERE t.subject_id = (k ->> 'subject_id')::text AND t.collection = (k ->> 'collection')::text AND t.scope = (k ->> 'scope')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_catalog_merge_state(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.cursor_artist, t.cursor_recording_key, t.groups_merged, t.passes_completed, t.singleton, t.tracks_superseded, t.updated_at)::text) FROM public.catalog_merge_state t WHERE t.singleton = (k ->> 'singleton')::boolean
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_catalog_work_links(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.linked_at, t.match_reason, t.matched_key, t.normalizer_version, t.track_id, t.wanted_track_id)::text) FROM public.catalog_work_links t WHERE t.wanted_track_id = (k ->> 'wanted_track_id')::uuid AND t.track_id = (k ->> 'track_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_cdn_tracks(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.cdn_path, t.created_at, t.id, t.quality, t.row_v, t.status, t.stored_quality, t.track_urn, t.updated_at)::text) FROM public.cdn_tracks t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_cmd_dead(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.args, t.at, t.id, t.kind, t.message, t.node, t.sqlstate)::text) FROM public.cmd_dead t WHERE t.node = (k ->> 'node')::text AND t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_diag_bundles(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.app_version, t.at, t.body, t.code, t.os)::text) FROM public.diag_bundles t WHERE t.code = (k ->> 'code')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_diag_events(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.app_version, t.at, t.body, t.fingerprint, t.id, t.install, t.kind, t.os, t.title, t.trace)::text) FROM public.diag_events t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_diag_issue_days(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.app_version, t.day, t.events, t.fingerprint, t.first_at, t.host, t.kind, t.last_at, t.row_v, t.title)::text) FROM public.diag_issue_days t WHERE t.fingerprint = (k ->> 'fingerprint')::text AND t.day = (k ->> 'day')::date AND t.host = (k ->> 'host')::text AND t.app_version = (k ->> 'app_version')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_diag_issue_origins(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.day, t.fingerprint, t.host, t.origin)::text) FROM public.diag_issue_origins t WHERE t.fingerprint = (k ->> 'fingerprint')::text AND t.day = (k ->> 'day')::date AND t.host = (k ->> 'host')::text AND t.origin = (k ->> 'origin')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_discover_promoted(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.active, t.created_at, t.entity_id, t.entity_type, t.id, t.note, t."position", t.row_v, t.updated_at)::text) FROM public.discover_promoted t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_discover_settings(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.id, t.row_v, t.show_star, t.star_limit, t.star_strategy, t.updated_at)::text) FROM public.discover_settings t WHERE t.id = (k ->> 'id')::smallint
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_discover_tag_counts(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.artist_count, t.refreshed_at, t.tag)::text) FROM public.discover_tag_counts t WHERE t.tag = (k ->> 'tag')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_disliked_tracks(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.created_at, t.id, t.row_v, t.sc_track_id, t.sc_user_id, t.track_data)::text) FROM public.disliked_tracks t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_enrich_calibration(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.calibrated, t.raw_bin_high, t.raw_bin_low, t.sample_count, t.source, t.updated_at)::text) FROM public.enrich_calibration t WHERE t.source = (k ->> 'source')::character varying(16) AND t.raw_bin_low = (k ->> 'raw_bin_low')::real AND t.raw_bin_high = (k ->> 'raw_bin_high')::real
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_entity_reads(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.entity_urn, t.last_read_at)::text) FROM public.entity_reads t WHERE t.entity_urn = (k ->> 'entity_urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_featured_items(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.active, t.created_at, t.id, t.row_v, t.sc_urn, t.type, t.weight)::text) FROM public.featured_items t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_job_status(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.available_at, t.dedup_key, t.kind, t.state, t.updated_at)::text) FROM public.job_status t WHERE t.kind = (k ->> 'kind')::text AND t.dedup_key = (k ->> 'dedup_key')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_link_requests(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.claim_token, t.created_at, t.error, t.expires_at, t.id, t.mode, t.row_v, t.source_session_id, t.status, t.target_session_id)::text) FROM public.link_requests t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_listening_history(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.artist_name, t.artist_urn, t.artwork_url, t.duration, t.id, t.played_at, t.sc_track_id, t.soundcloud_user_id, t.title)::text) FROM public.listening_history t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_login_requests(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.code_verifier, t.created_at, t.error, t.expires_at, t.id, t.oauth_app_id, t.profile_ok, t.redirect_url, t.result_session_id, t.retry_count, t.row_v, t.state, t.status, t.step, t.target_session_id, t.username)::text) FROM public.login_requests t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_lyrics_cache(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.content_generation, t.content_hash, t.created_at, t.embedded_at, t.embedding_state, t.language, t.language_confidence, t.plain_source, t.plain_text, t.sc_track_id, t.source, t.synced_lrc, t.synced_source, t.synced_version, t.track_id, t.updated_at)::text) FROM public.lyrics_cache t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_lyrics_embedding_wire_state(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.completed_at, t.created_at, t.first_publish_attempt_at, t.lyrics_content_generation, t.lyrics_created_at, t.publish_acknowledged_at, t.publish_retry_until, t.quarantine_reason, t.reopen_count, t.request_language, t.request_message_id, t.request_sha256, t.request_text, t.request_version, t.result_consumer, t.result_kind, t.result_lease_expires_at, t.result_lease_id, t.result_published_at, t.result_rank, t.result_reason, t.result_stream, t.result_stream_sequence, t.result_timeouts, t.sc_track_id, t.status, t.updated_at)::text) FROM public.lyrics_embedding_wire_state t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_lyrics_lookup_backfill_state(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.completed, t.created_at, t.cursor_id, t.rows_created, t.rows_seen, t.singleton, t.updated_at)::text) FROM public.lyrics_lookup_backfill_state t WHERE t.singleton = (k ->> 'singleton')::boolean
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_lyrics_lookup_state(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.algorithm_version, t.attempts, t.claim_expires_at, t.claim_job_generation, t.claim_job_id, t.claim_job_lease_id, t.claim_state_generation, t.created_at, t.failure_streak, t.generation, t.input_artist, t.input_duration_ms, t.input_genius_song_id, t.input_genius_url, t.input_release_date, t.input_sc_created_at, t.input_title, t.last_attempt_at, t.last_error, t.last_outcome, t.miss_streak, t.next_run_at, t.priority, t.retry_after_at, t.sc_track_id, t.source_set_version, t.status, t.track_id, t.updated_at, t.wake_durable_at, t.wake_generation, t.wake_message_id)::text) FROM public.lyrics_lookup_state t WHERE t.track_id = (k ->> 'track_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_oauth_app_request_cooldowns(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.failure_count, t.oauth_app_id, t.retry_at, t.row_v, t.updated_at)::text) FROM public.oauth_app_request_cooldowns t WHERE t.oauth_app_id = (k ->> 'oauth_app_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_oauth_app_token_issuance_reservations(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.client_id, t.id, t.oauth_app_id, t.released_at, t.reserved_at)::text) FROM public.oauth_app_token_issuance_reservations t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_oauth_app_token_refresh_state(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.lease_expires_at, t.lease_id, t.oauth_app_id, t.retry_at, t.updated_at)::text) FROM public.oauth_app_token_refresh_state t WHERE t.oauth_app_id = (k ->> 'oauth_app_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_oauth_app_tokens(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.access_token, t.expires_at, t.generation, t.last_refresh_error, t.last_used_at, t.oauth_app_id, t.refresh_attempts, t.refresh_token, t.refreshed_at, t.scope)::text) FROM public.oauth_app_tokens t WHERE t.oauth_app_id = (k ->> 'oauth_app_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_oauth_apps(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.active, t.client_id, t.client_secret, t.created_at, t.id, t.name, t.redirect_uri, t.row_v, t.updated_at)::text) FROM public.oauth_apps t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_pipeline_event_receipts(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.consumer, t.event_published_at, t.processed_at, t.stream, t.stream_sequence)::text) FROM public.pipeline_event_receipts t WHERE t.consumer = (k ->> 'consumer')::character varying(96) AND t.stream = (k ->> 'stream')::character varying(128) AND t.stream_sequence = (k ->> 'stream_sequence')::bigint AND t.event_published_at = (k ->> 'event_published_at')::timestamp with time zone
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_playlist_legacy_membership_intents(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.archive_id, t.archived_at, t.classification, t.legacy_desired_revision, t.legacy_synced_revision, t.legacy_user_id, t.local_snapshot_id, t.playlist_urn, t.prior_classification, t.queue_created_at, t.queue_failed_at, t.queue_generation, t.queue_id, t.queue_last_error, t.queue_next_run_at, t.queue_payload, t.queue_retry_count, t.remote_attempted_generation, t.remote_completed_generation, t.remote_result, t.resolved_at, t.source)::text) FROM public.playlist_legacy_membership_intents t WHERE t.archive_id = (k ->> 'archive_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_playlist_membership_operations(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.accepted_projection_revision, t.actor_sc_user_id, t.base_baseline_generation, t.base_observation_id, t.boundary, t.conflict_code, t.created_at, t.expected_projection_revision, t.idempotency_key, t.kind, t.left_anchor_track_id, t.operation_id, t.ordered_track_ids, t.outcome, t.playlist_urn, t.request_fingerprint, t.resolved_at, t.right_anchor_track_id, t.sequence, t.track_id)::text) FROM public.playlist_membership_operations t WHERE t.operation_id = (k ->> 'operation_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_playlist_membership_state(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.baseline_generation, t.baseline_observation_id, t.candidate_fingerprint, t.committed_operation_sequence, t.conflict_code, t.last_error, t.last_operation_sequence, t.latest_observation_id, t.next_reconcile_at, t.owner_sc_user_id, t.playlist_urn, t.projection_revision, t.projection_track_count, t.reconcile_failure_streak, t.reconcile_generation, t.remote_applied_at, t.remote_apply_fingerprint, t.remote_apply_generation, t.sync_status, t.updated_at)::text) FROM public.playlist_membership_state t WHERE t.playlist_urn = (k ->> 'playlist_urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_playlist_reconcile_runs(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.candidate_fingerprint, t.captured_baseline_generation, t.captured_through_operation_sequence, t.completed_at, t.decision, t.id, t.job_generation, t.job_id, t.mode, t.observation_id, t.playlist_urn, t.reason, t.reconcile_generation, t.started_at)::text) FROM public.playlist_reconcile_runs t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_playlist_remote_observations(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.all_items_identified, t.authority, t.declared_track_count, t.error_kind, t.id, t.observed_at, t.observed_track_count, t.outcome, t.pagination_complete, t.playlist_urn, t.retry_at, t.sc_last_modified, t.snapshot_id)::text) FROM public.playlist_remote_observations t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_playlist_remote_snapshot_tracks(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t."position", t.sc_track_id, t.snapshot_id)::text) FROM public.playlist_remote_snapshot_tracks t WHERE t.snapshot_id = (k ->> 'snapshot_id')::uuid AND t."position" = (k ->> 'position')::integer
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_playlist_remote_snapshots(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.content_fingerprint, t.created_at, t.id, t.playlist_urn, t.track_count)::text) FROM public.playlist_remote_snapshots t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_playlist_sweep_cursor(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.id, t.last_owner_sc_user_id, t.updated_at)::text) FROM public.playlist_sweep_cursor t WHERE t.id = (k ->> 'id')::boolean
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_playlist_track_projection(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.playlist_urn, t."position", t.sc_track_id)::text) FROM public.playlist_track_projection t WHERE t.playlist_urn = (k ->> 'playlist_urn')::text AND t."position" = (k ->> 'position')::integer
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_playlists(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.artwork_url, t.created_at, t.deleted_at, t.description, t.duration_ms, t.genre, t.kind, t.label_name, t.likes_count_sc, t.owner_sc_user_id, t.owner_urn, t.owner_username, t.permalink_url, t.playlist_type, t.release_date, t.release_year, t.reposts_count_sc, t.sc_created_at, t.sc_desired, t.sc_last_modified, t.sc_metadata, t.sc_mutation_observation, t.sc_observation, t.sc_playlist_id, t.sc_synced_at, t.sc_write_confirmed, t.sharing, t.tags, t.title, t.title_normalized, t.track_count, t.updated_at, t.urn)::text) FROM public.playlists t WHERE t.urn = (k ->> 'urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_recommendation_quality_models(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.examples, t.feature_means, t.feature_scales, t.intercept, t.positives, t.train_accuracy, t.trained_at, t.version, t.weights)::text) FROM public.recommendation_quality_models t WHERE t.version = (k ->> 'version')::bigint
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_repl_checksums(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.at, t.hash, t.node, t.range, t.tbl)::text) FROM public.repl_checksums t WHERE t.node = (k ->> 'node')::text AND t.tbl = (k ->> 'tbl')::text AND t.range = (k ->> 'range')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_repl_tombstones(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.at, t.pk, t.tbl, t.v)::text) FROM public.repl_tombstones t WHERE t.tbl = (k ->> 'tbl')::text AND t.pk = (k ->> 'pk')::jsonb AND t.v = (k ->> 'v')::bigint
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_sc_egress_health(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.channel, t.open_until, t.opened_at, t.opened_by, t.row_v)::text) FROM public.sc_egress_health t WHERE t.channel = (k ->> 'channel')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_sc_track_counters(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.comment_count, t.fetched_at, t.likes_count, t.play_count, t.reposts_count, t.sc_track_id)::text) FROM public.sc_track_counters t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_search_terms(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.latin, t.ndoc, t.word)::text) FROM public.search_terms t WHERE t.word = (k ->> 'word')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_sessions(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.created_at, t.id, t.row_v, t.soundcloud_connection_id, t.updated_at)::text) FROM public.sessions t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_soundcloud_connections(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.access_token, t.created_at, t.expires_at, t.first_refresh_rejection_at, t.id, t.last_refresh_attempt_at, t.last_refresh_error, t.last_refresh_error_kind, t.last_refresh_success_at, t.oauth_app_id, t.refresh_failure_count, t.refresh_generation, t.refresh_lease_expires_at, t.refresh_lease_id, t.refresh_rejection_count, t.refresh_token, t.retry_at, t.scope, t.soundcloud_user_id, t.token_v, t.updated_at, t.username)::text) FROM public.soundcloud_connections t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_star_track_tiers(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.align_due, t.audio_due, t.embed_due, t.enrich_due, t.lookup_due, t.sc_track_id, t.tier, t.tiered_at, t.upgrade_from, t.upgrade_tried_at)::text) FROM public.star_track_tiers t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_storage_event_repairs(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.repaired_at, t.sc_track_id, t.uploaded_at)::text) FROM public.storage_event_repairs t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_storage_event_state(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.event_published_at, t.sc_track_id, t.stream, t.stream_sequence, t.transcription_generation, t.updated_at, t.uploaded_generation)::text) FROM public.storage_event_state t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_subscription_snapshot_state(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.initialized_at, t.singleton)::text) FROM public.subscription_snapshot_state t WHERE t.singleton = (k ->> 'singleton')::boolean
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_subscriptions(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.exp_date, t.row_v, t.user_urn)::text) FROM public.subscriptions t WHERE t.user_urn = (k ->> 'user_urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_sync_queue(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.action_type, t.cancelled, t.created_at, t.dead, t.exec_generation, t.failed_at, t.generation, t.id, t.intent_v, t.last_error, t.lease_generation, t.lease_id, t.locked_at, t.next_run_at, t.payload, t.remote_attempted_generation, t.remote_completed_generation, t.remote_result, t.retry_count, t.target_urn, t.user_id)::text) FROM public.sync_queue t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_sync_queue_action_pauses(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.action_type, t.oauth_app_id, t.paused_until, t.reason, t.strikes, t.updated_at)::text) FROM public.sync_queue_action_pauses t WHERE t.oauth_app_id = (k ->> 'oauth_app_id')::uuid AND t.action_type = (k ->> 'action_type')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_taste_model_versions(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.active, t.applied_at, t.collection, t.created_at, t.dim, t.input_object, t.items_count, t.metrics, t.pooling, t.refreshed_through, t.trained_at, t.users_count, t.version)::text) FROM public.taste_model_versions t WHERE t.version = (k ->> 'version')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_taste_schedule(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.next_export_at, t.next_refresh_at, t.singleton)::text) FROM public.taste_schedule t WHERE t.singleton = (k ->> 'singleton')::boolean
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_track_artist_blocks(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.artist_id, t.created_at, t.note, t.row_v, t.track_id)::text) FROM public.track_artist_blocks t WHERE t.track_id = (k ->> 'track_id')::uuid AND t.artist_id = (k ->> 'artist_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_track_artists(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.artist_id, t.confidence, t.evidence, t."position", t.role, t.source, t.track_id)::text) FROM public.track_artists t WHERE t.track_id = (k ->> 'track_id')::uuid AND t.artist_id = (k ->> 'artist_id')::uuid AND t.role = (k ->> 'role')::character varying(16)
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_track_comments(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.body, t.created_at, t.id, t.sc_comment_id, t.sc_created_at, t.sc_track_id, t.synced_at, t.track_position_ms, t.user_urn)::text) FROM public.track_comments t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_track_work_aliases(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.alias_key, t.track_id)::text) FROM public.track_work_aliases t WHERE t.track_id = (k ->> 'track_id')::uuid AND t.alias_key = (k ->> 'alias_key')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_tracks(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.album_id, t.album_position, t.artwork_url, t.audio_fingerprint, t.canonical_track_id, t.comments_count_sc, t.cover_of_artist_id, t.created_at, t.deleted_at, t.description, t.duration_ms, t.duration_resolve_attempts, t.duration_resolve_retry_at, t.enrich_attempts, t.enrich_confidence, t.enrich_error, t.enrich_locked_at, t.enrich_next_run_at, t.enrich_source, t.enrich_state, t.enriched_at, t.genius_song_id, t.genius_url, t.genre, t.hq_upgrade_attempts, t.hq_upgrade_last_at, t.hq_upgrade_pending, t.id, t.index_attempts, t.index_priority, t.index_state, t.indexed_at, t.is_cover, t.isrc, t.language, t.language_confidence, t.likes_count_sc, t.metadata_artist, t.needs_duration_resolve, t.permalink_url, t.pipeline_held, t.play_count_sc, t.primary_artist_id, t.quality_model_version, t.quality_score, t.recording_key, t.release_date, t.release_year, t.reposts_count_sc, t.s3_missing_at, t.s3_verified_at, t.sc_created_at, t.sc_desired, t.sc_last_modified, t.sc_metadata, t.sc_mutation_observation, t.sc_observation, t.sc_synced_at, t.sc_track_id, t.sc_write_confirmed, t.sharing, t.storage_attempts, t.storage_priority, t.storage_quality, t.storage_state, t.superseded_by, t.tags, t.title, t.title_normalized, t.transcribe_at, t.transcribe_state, t.updated_at, t.upload_kind, t.uploader_avatar_url, t.uploader_sc_user_id, t.uploader_urn, t.uploader_username, t.urn, t.waveform_url, t.work_key, t.work_normalizer_version)::text) FROM public.tracks t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_transcription_sync_versions(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.first_seen_at, t.last_seen_at, t.sync_version)::text) FROM public.transcription_sync_versions t WHERE t.sync_version = (k ->> 'sync_version')::character varying(128)
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_transcription_wire_state(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.aligned_share, t.attempt, t.completed_at, t.confidence, t.dispatched_at, t.lines_total, t.lines_unplaced, t.placed_share, t.quarantine_reason, t.reason, t.reopen_count, t.reopened_for_sync_version, t.result_language, t.result_published_at, t.result_rank, t.result_stream_sequence, t.result_timeouts, t.sc_track_id, t.status, t.sync_version, t.updated_at, t.upload_generation)::text) FROM public.transcription_wire_state t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_user_auras(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.aura_id, t.custom_hex, t.row_v, t.updated_at, t.user_urn)::text) FROM public.user_auras t WHERE t.user_urn = (k ->> 'user_urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_user_blocked_artists(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.avatar_url, t.created_at, t.kind, t.name, t.row_v, t.sc_user_id, t.sc_user_ids, t.target_id)::text) FROM public.user_blocked_artists t WHERE t.sc_user_id = (k ->> 'sc_user_id')::text AND t.kind = (k ->> 'kind')::text AND t.target_id = (k ->> 'target_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_user_collection_sync(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.collection, t.synced_at, t.user_id)::text) FROM public.user_collection_sync t WHERE t.user_id = (k ->> 'user_id')::text AND t.collection = (k ->> 'collection')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_user_events(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.created_at, t.event_type, t.id, t.position_pct, t.sc_track_id, t.sc_user_id, t.weight)::text) FROM public.user_events t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_user_followers(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.created_at, t.progress, t.synced_at, t.target_user_urn, t.user_id)::text) FROM public.user_followers t WHERE t.user_id = (k ->> 'user_id')::text AND t.target_user_urn = (k ->> 'target_user_urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_user_followings(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.created_at, t.progress, t.synced_at, t.synced_for_v, t.target_user_urn, t.user_id, t.wanted_state, t.wanted_v)::text) FROM public.user_followings t WHERE t.user_id = (k ->> 'user_id')::text AND t.target_user_urn = (k ->> 'target_user_urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_user_like_ranks(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.like_created_at, t.rank, t.row_v, t.sc_track_id, t.user_id)::text) FROM public.user_like_ranks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_user_likes_playlists(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.created_at, t.playlist_urn, t.progress, t.synced_at, t.synced_for_v, t.user_id, t.wanted_state, t.wanted_v)::text) FROM public.user_likes_playlists t WHERE t.user_id = (k ->> 'user_id')::text AND t.playlist_urn = (k ->> 'playlist_urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_user_likes_tracks(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.created_at, t.liked_at, t.progress, t.sc_track_id, t.synced_at, t.synced_for_v, t.user_id, t.wanted_state, t.wanted_v)::text) FROM public.user_likes_tracks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_user_owned_playlists(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.created_at, t.playlist_urn, t.progress, t.synced_at, t.user_id)::text) FROM public.user_owned_playlists t WHERE t.user_id = (k ->> 'user_id')::text AND t.playlist_urn = (k ->> 'playlist_urn')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_user_owned_tracks(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.created_at, t.progress, t.sc_track_id, t.synced_at, t.user_id)::text) FROM public.user_owned_tracks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_user_profiles(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.profile_json, t.sc_observation, t.soundcloud_user_id, t.synced_at)::text) FROM public.user_profiles t WHERE t.soundcloud_user_id = (k ->> 'soundcloud_user_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_user_superlike_ranks(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.rank, t.row_v, t.sc_track_id, t.superlike_created_at, t.user_id)::text) FROM public.user_superlike_ranks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_user_superlikes(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.created_at, t.like_created_at, t.row_v, t.sc_track_id, t.user_id)::text) FROM public.user_superlikes t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_user_taste_vectors(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.sc_user_id, t.updated_at, t.vec, t.version)::text) FROM public.user_taste_vectors t WHERE t.sc_user_id = (k ->> 'sc_user_id')::text AND t.version = (k ->> 'version')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_user_web_profiles(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.profiles, t.sc_observation, t.sc_user_id, t.synced_at)::text) FROM public.user_web_profiles t WHERE t.sc_user_id = (k ->> 'sc_user_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_users(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.avatar_url, t.city, t.comments_count, t.country, t.created_at, t.description, t.first_name, t.followers_count, t.followings_count, t.full_name, t.kind, t.last_name, t.permalink, t.permalink_url, t.playlists_count, t.reposts_count, t.sc_created_at, t.sc_last_modified, t.sc_observation, t.sc_synced_at, t.sc_user_id, t.tracks_count, t.updated_at, t.urn, t.username, t.username_normalized, t.verified)::text) FROM public.users t WHERE t.sc_user_id = (k ->> 'sc_user_id')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_wanted_track_albums(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.album_id, t."position", t.wanted_track_id)::text) FROM public.wanted_track_albums t WHERE t.wanted_track_id = (k ->> 'wanted_track_id')::uuid AND t.album_id = (k ->> 'album_id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_wanted_track_artists(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.artist_id, t."position", t.role, t.wanted_track_id)::text) FROM public.wanted_track_artists t WHERE t.wanted_track_id = (k ->> 'wanted_track_id')::uuid AND t.artist_id = (k ->> 'artist_id')::uuid AND t.role = (k ->> 'role')::character varying(16)
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_wanted_track_work_aliases(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.alias_key, t.wanted_track_id)::text) FROM public.wanted_track_work_aliases t WHERE t.wanted_track_id = (k ->> 'wanted_track_id')::uuid AND t.alias_key = (k ->> 'alias_key')::text
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash_wanted_tracks(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.discovered_at, t.duration_ms, t.external_id, t.fetched_at, t.id, t.isrc, t.normalized_title, t.primary_artist_id, t.recording_key, t.release_year, t.resolve_attempts, t.resolve_error, t.resolve_locked_at, t.resolve_next_run_at, t.source, t.status, t.title, t.track_id, t.updated_at, t.work_key, t.work_normalizer_version, t.work_reconciled_at)::text) FROM public.wanted_tracks t WHERE t.id = (k ->> 'id')::uuid
$f$;

CREATE OR REPLACE FUNCTION public.scd_row_hash(tbl text, k jsonb)
RETURNS text
LANGUAGE plpgsql STABLE
AS $f$
BEGIN
    CASE tbl
        WHEN 'admin_maintenance_runs' THEN
            RETURN public.scd_row_hash_admin_maintenance_runs(k);
        WHEN 'ai_resolver_budget' THEN
            RETURN public.scd_row_hash_ai_resolver_budget(k);
        WHEN 'ai_resolver_cache' THEN
            RETURN public.scd_row_hash_ai_resolver_cache(k);
        WHEN 'album_artists' THEN
            RETURN public.scd_row_hash_album_artists(k);
        WHEN 'album_tracks' THEN
            RETURN public.scd_row_hash_album_tracks(k);
        WHEN 'albums' THEN
            RETURN public.scd_row_hash_albums(k);
        WHEN 'artist_attribution_revalidation' THEN
            RETURN public.scd_row_hash_artist_attribution_revalidation(k);
        WHEN 'artist_colike' THEN
            RETURN public.scd_row_hash_artist_colike(k);
        WHEN 'artist_coplay' THEN
            RETURN public.scd_row_hash_artist_coplay(k);
        WHEN 'artist_sc_accounts' THEN
            RETURN public.scd_row_hash_artist_sc_accounts(k);
        WHEN 'artist_socials' THEN
            RETURN public.scd_row_hash_artist_socials(k);
        WHEN 'artists' THEN
            RETURN public.scd_row_hash_artists(k);
        WHEN 'audio_index_wire_state' THEN
            RETURN public.scd_row_hash_audio_index_wire_state(k);
        WHEN 'background_job_enqueues' THEN
            RETURN public.scd_row_hash_background_job_enqueues(k);
        WHEN 'background_job_failures' THEN
            RETURN public.scd_row_hash_background_job_failures(k);
        WHEN 'background_jobs' THEN
            RETURN public.scd_row_hash_background_jobs(k);
        WHEN 'background_schedules' THEN
            RETURN public.scd_row_hash_background_schedules(k);
        WHEN 'catalog_audience' THEN
            RETURN public.scd_row_hash_catalog_audience(k);
        WHEN 'catalog_collection_cursors' THEN
            RETURN public.scd_row_hash_catalog_collection_cursors(k);
        WHEN 'catalog_collection_seen' THEN
            RETURN public.scd_row_hash_catalog_collection_seen(k);
        WHEN 'catalog_collection_sync' THEN
            RETURN public.scd_row_hash_catalog_collection_sync(k);
        WHEN 'catalog_merge_state' THEN
            RETURN public.scd_row_hash_catalog_merge_state(k);
        WHEN 'catalog_work_links' THEN
            RETURN public.scd_row_hash_catalog_work_links(k);
        WHEN 'cdn_tracks' THEN
            RETURN public.scd_row_hash_cdn_tracks(k);
        WHEN 'cmd_dead' THEN
            RETURN public.scd_row_hash_cmd_dead(k);
        WHEN 'diag_bundles' THEN
            RETURN public.scd_row_hash_diag_bundles(k);
        WHEN 'diag_events' THEN
            RETURN public.scd_row_hash_diag_events(k);
        WHEN 'diag_issue_days' THEN
            RETURN public.scd_row_hash_diag_issue_days(k);
        WHEN 'diag_issue_origins' THEN
            RETURN public.scd_row_hash_diag_issue_origins(k);
        WHEN 'discover_promoted' THEN
            RETURN public.scd_row_hash_discover_promoted(k);
        WHEN 'discover_settings' THEN
            RETURN public.scd_row_hash_discover_settings(k);
        WHEN 'discover_tag_counts' THEN
            RETURN public.scd_row_hash_discover_tag_counts(k);
        WHEN 'disliked_tracks' THEN
            RETURN public.scd_row_hash_disliked_tracks(k);
        WHEN 'enrich_calibration' THEN
            RETURN public.scd_row_hash_enrich_calibration(k);
        WHEN 'entity_reads' THEN
            RETURN public.scd_row_hash_entity_reads(k);
        WHEN 'featured_items' THEN
            RETURN public.scd_row_hash_featured_items(k);
        WHEN 'job_status' THEN
            RETURN public.scd_row_hash_job_status(k);
        WHEN 'link_requests' THEN
            RETURN public.scd_row_hash_link_requests(k);
        WHEN 'listening_history' THEN
            RETURN public.scd_row_hash_listening_history(k);
        WHEN 'login_requests' THEN
            RETURN public.scd_row_hash_login_requests(k);
        WHEN 'lyrics_cache' THEN
            RETURN public.scd_row_hash_lyrics_cache(k);
        WHEN 'lyrics_embedding_wire_state' THEN
            RETURN public.scd_row_hash_lyrics_embedding_wire_state(k);
        WHEN 'lyrics_lookup_backfill_state' THEN
            RETURN public.scd_row_hash_lyrics_lookup_backfill_state(k);
        WHEN 'lyrics_lookup_state' THEN
            RETURN public.scd_row_hash_lyrics_lookup_state(k);
        WHEN 'oauth_app_request_cooldowns' THEN
            RETURN public.scd_row_hash_oauth_app_request_cooldowns(k);
        WHEN 'oauth_app_token_issuance_reservations' THEN
            RETURN public.scd_row_hash_oauth_app_token_issuance_reservations(k);
        WHEN 'oauth_app_token_refresh_state' THEN
            RETURN public.scd_row_hash_oauth_app_token_refresh_state(k);
        WHEN 'oauth_app_tokens' THEN
            RETURN public.scd_row_hash_oauth_app_tokens(k);
        WHEN 'oauth_apps' THEN
            RETURN public.scd_row_hash_oauth_apps(k);
        WHEN 'pipeline_event_receipts' THEN
            RETURN public.scd_row_hash_pipeline_event_receipts(k);
        WHEN 'playlist_legacy_membership_intents' THEN
            RETURN public.scd_row_hash_playlist_legacy_membership_intents(k);
        WHEN 'playlist_membership_operations' THEN
            RETURN public.scd_row_hash_playlist_membership_operations(k);
        WHEN 'playlist_membership_state' THEN
            RETURN public.scd_row_hash_playlist_membership_state(k);
        WHEN 'playlist_reconcile_runs' THEN
            RETURN public.scd_row_hash_playlist_reconcile_runs(k);
        WHEN 'playlist_remote_observations' THEN
            RETURN public.scd_row_hash_playlist_remote_observations(k);
        WHEN 'playlist_remote_snapshot_tracks' THEN
            RETURN public.scd_row_hash_playlist_remote_snapshot_tracks(k);
        WHEN 'playlist_remote_snapshots' THEN
            RETURN public.scd_row_hash_playlist_remote_snapshots(k);
        WHEN 'playlist_sweep_cursor' THEN
            RETURN public.scd_row_hash_playlist_sweep_cursor(k);
        WHEN 'playlist_track_projection' THEN
            RETURN public.scd_row_hash_playlist_track_projection(k);
        WHEN 'playlists' THEN
            RETURN public.scd_row_hash_playlists(k);
        WHEN 'recommendation_quality_models' THEN
            RETURN public.scd_row_hash_recommendation_quality_models(k);
        WHEN 'repl_checksums' THEN
            RETURN public.scd_row_hash_repl_checksums(k);
        WHEN 'repl_tombstones' THEN
            RETURN public.scd_row_hash_repl_tombstones(k);
        WHEN 'sc_egress_health' THEN
            RETURN public.scd_row_hash_sc_egress_health(k);
        WHEN 'sc_track_counters' THEN
            RETURN public.scd_row_hash_sc_track_counters(k);
        WHEN 'search_terms' THEN
            RETURN public.scd_row_hash_search_terms(k);
        WHEN 'sessions' THEN
            RETURN public.scd_row_hash_sessions(k);
        WHEN 'soundcloud_connections' THEN
            RETURN public.scd_row_hash_soundcloud_connections(k);
        WHEN 'star_track_tiers' THEN
            RETURN public.scd_row_hash_star_track_tiers(k);
        WHEN 'storage_event_repairs' THEN
            RETURN public.scd_row_hash_storage_event_repairs(k);
        WHEN 'storage_event_state' THEN
            RETURN public.scd_row_hash_storage_event_state(k);
        WHEN 'subscription_snapshot_state' THEN
            RETURN public.scd_row_hash_subscription_snapshot_state(k);
        WHEN 'subscriptions' THEN
            RETURN public.scd_row_hash_subscriptions(k);
        WHEN 'sync_queue' THEN
            RETURN public.scd_row_hash_sync_queue(k);
        WHEN 'sync_queue_action_pauses' THEN
            RETURN public.scd_row_hash_sync_queue_action_pauses(k);
        WHEN 'taste_model_versions' THEN
            RETURN public.scd_row_hash_taste_model_versions(k);
        WHEN 'taste_schedule' THEN
            RETURN public.scd_row_hash_taste_schedule(k);
        WHEN 'track_artist_blocks' THEN
            RETURN public.scd_row_hash_track_artist_blocks(k);
        WHEN 'track_artists' THEN
            RETURN public.scd_row_hash_track_artists(k);
        WHEN 'track_comments' THEN
            RETURN public.scd_row_hash_track_comments(k);
        WHEN 'track_work_aliases' THEN
            RETURN public.scd_row_hash_track_work_aliases(k);
        WHEN 'tracks' THEN
            RETURN public.scd_row_hash_tracks(k);
        WHEN 'transcription_sync_versions' THEN
            RETURN public.scd_row_hash_transcription_sync_versions(k);
        WHEN 'transcription_wire_state' THEN
            RETURN public.scd_row_hash_transcription_wire_state(k);
        WHEN 'user_auras' THEN
            RETURN public.scd_row_hash_user_auras(k);
        WHEN 'user_blocked_artists' THEN
            RETURN public.scd_row_hash_user_blocked_artists(k);
        WHEN 'user_collection_sync' THEN
            RETURN public.scd_row_hash_user_collection_sync(k);
        WHEN 'user_events' THEN
            RETURN public.scd_row_hash_user_events(k);
        WHEN 'user_followers' THEN
            RETURN public.scd_row_hash_user_followers(k);
        WHEN 'user_followings' THEN
            RETURN public.scd_row_hash_user_followings(k);
        WHEN 'user_like_ranks' THEN
            RETURN public.scd_row_hash_user_like_ranks(k);
        WHEN 'user_likes_playlists' THEN
            RETURN public.scd_row_hash_user_likes_playlists(k);
        WHEN 'user_likes_tracks' THEN
            RETURN public.scd_row_hash_user_likes_tracks(k);
        WHEN 'user_owned_playlists' THEN
            RETURN public.scd_row_hash_user_owned_playlists(k);
        WHEN 'user_owned_tracks' THEN
            RETURN public.scd_row_hash_user_owned_tracks(k);
        WHEN 'user_profiles' THEN
            RETURN public.scd_row_hash_user_profiles(k);
        WHEN 'user_superlike_ranks' THEN
            RETURN public.scd_row_hash_user_superlike_ranks(k);
        WHEN 'user_superlikes' THEN
            RETURN public.scd_row_hash_user_superlikes(k);
        WHEN 'user_taste_vectors' THEN
            RETURN public.scd_row_hash_user_taste_vectors(k);
        WHEN 'user_web_profiles' THEN
            RETURN public.scd_row_hash_user_web_profiles(k);
        WHEN 'users' THEN
            RETURN public.scd_row_hash_users(k);
        WHEN 'wanted_track_albums' THEN
            RETURN public.scd_row_hash_wanted_track_albums(k);
        WHEN 'wanted_track_artists' THEN
            RETURN public.scd_row_hash_wanted_track_artists(k);
        WHEN 'wanted_track_work_aliases' THEN
            RETURN public.scd_row_hash_wanted_track_work_aliases(k);
        WHEN 'wanted_tracks' THEN
            RETURN public.scd_row_hash_wanted_tracks(k);
        ELSE
            RAISE EXCEPTION 'public.scd_row_hash: unknown table %', tbl;
    END CASE;
END
$f$;

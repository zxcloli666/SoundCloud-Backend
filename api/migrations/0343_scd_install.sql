-- no-transaction
BEGIN;
SET LOCAL lock_timeout = '5s';
CREATE SEQUENCE IF NOT EXISTS public.scd_conflict_cdn_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_conflict_discover_promoted;
CREATE SEQUENCE IF NOT EXISTS public.scd_conflict_disliked_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_conflict_oauth_apps;
CREATE SEQUENCE IF NOT EXISTS public.scd_conflict_sync_queue;
CREATE SEQUENCE IF NOT EXISTS public.scd_core_col_artists;
CREATE SEQUENCE IF NOT EXISTS public.scd_core_col_lyrics_cache;
CREATE SEQUENCE IF NOT EXISTS public.scd_core_col_playlists;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_admin_catalog_requests;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_admin_maintenance_runs;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_ai_resolver_budget;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_ai_resolver_cache;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_album_artists;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_album_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_albums;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_artist_attribution_revalidation;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_artist_colike;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_artist_coplay;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_artist_sc_accounts;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_artist_socials;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_artists;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_audio_index_wire_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_background_job_enqueues;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_background_job_failures;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_background_jobs;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_background_schedules;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_catalog_audience;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_catalog_collection_cursors;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_catalog_collection_seen;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_catalog_collection_sync;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_catalog_merge_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_catalog_work_links;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_cmd_dead;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_discover_tag_counts;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_enrich_calibration;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_job_status;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_lyrics_cache;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_lyrics_embedding_wire_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_lyrics_lookup_backfill_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_lyrics_lookup_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_oauth_app_token_issuance_reservations;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_oauth_app_token_refresh_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_oauth_app_tokens;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_pipeline_event_receipts;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlist_legacy_membership_intents;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlist_membership_operations;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlist_membership_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlist_reconcile_runs;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlist_remote_observations;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlist_remote_snapshot_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlist_remote_snapshots;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlist_sweep_cursor;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlist_track_projection;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlists;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_recommendation_quality_models;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_search_terms;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_star_track_tiers;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_storage_event_repairs;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_storage_event_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_subscription_snapshot_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_sync_queue_action_pauses;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_taste_model_versions;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_taste_schedule;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_track_artists;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_track_comments;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_track_work_aliases;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_transcription_sync_versions;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_transcription_wire_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_user_collection_sync;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_user_followers;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_user_owned_playlists;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_user_owned_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_user_profiles;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_user_taste_vectors;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_user_web_profiles;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_users;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_wanted_track_albums;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_wanted_track_artists;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_wanted_track_work_aliases;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_wanted_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_intent_sync_queue;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_intent_user_followings;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_intent_user_likes_playlists;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_intent_user_likes_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_result_soundcloud_connections;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_result_sync_queue;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_result_user_followings;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_result_user_likes_playlists;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_result_user_likes_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_orphan_oauth_app_request_cooldowns;
CREATE SEQUENCE IF NOT EXISTS public.scd_orphan_sessions;
CREATE SEQUENCE IF NOT EXISTS public.scd_orphan_soundcloud_connections;
CREATE SEQUENCE IF NOT EXISTS public.scd_orphan_track_artist_blocks;
CREATE SEQUENCE IF NOT EXISTS public.scd_purge_block_soundcloud_connections;
CREATE SEQUENCE IF NOT EXISTS public.scd_purge_block_sync_queue;
CREATE SEQUENCE IF NOT EXISTS public.scd_purge_block_user_followings;
CREATE SEQUENCE IF NOT EXISTS public.scd_purge_block_user_likes_playlists;
CREATE SEQUENCE IF NOT EXISTS public.scd_purge_block_user_likes_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_discover_promoted;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_disliked_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_sc_egress_health;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_soundcloud_connections;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_subscriptions;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_sync_queue;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_track_artist_blocks;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_user_blocked_artists;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_user_followings;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_user_like_ranks;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_user_likes_playlists;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_user_likes_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_user_superlike_ranks;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_user_superlikes;
CREATE SEQUENCE IF NOT EXISTS public.scd_restrict_oauth_apps;

CREATE OR REPLACE FUNCTION public.scd_guard()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF 'cmd' = ANY (TG_ARGV) AND coalesce(current_setting('scd.cmd', true), '') <> '' THEN
        RETURN NULL;
    END IF;
    IF coalesce(current_setting('scd.writer', true), '') = ANY (TG_ARGV[1:]) THEN
        RETURN NULL;
    END IF;
    PERFORM nextval(TG_ARGV[0]::regclass);
    IF current_setting('scd.guard', true) = 'enforce' THEN
        RAISE EXCEPTION 'scd guard: % on public.% by writer %', TG_OP, TG_TABLE_NAME,
            coalesce(nullif(current_setting('scd.writer', true), ''), 'none')
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    RETURN NULL;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_kinds()
RETURNS TABLE (kind text, optimistic boolean)
LANGUAGE sql STABLE
AS $f$
VALUES
    ('app_token.reject', true),
    ('cold_refresh.clear', true),
    ('comment.post', true),
    ('connection.drop_orphan', false),
    ('connection.login', true),
    ('job.enqueue', false),
    ('lyrics.wake', true),
    ('match.album_track', true),
    ('match.genius_song', true),
    ('match.sc_account', true),
    ('match.wanted_link', true),
    ('observe.playlists', true),
    ('observe.profile', true),
    ('observe.tracks', true),
    ('observe.users', true),
    ('owned.delete', true),
    ('owned.insert', true),
    ('playlist.delete', true),
    ('playlist.edit', true),
    ('playlist.membership_op', true),
    ('sync_queue.purge', false),
    ('sync_queue.revive', false),
    ('track.delete', true),
    ('track.edit', true),
    ('track.release_held', true),
    ('track.s3_state', false),
    ('track.storage_lost', true),
    ('track.too_long', true)
$f$;

CREATE OR REPLACE FUNCTION public.scd_table_class(tbl text)
RETURNS text
LANGUAGE sql STABLE
AS $f$
SELECT v.cls FROM (VALUES
    ('admin_catalog_requests', 'P'),
    ('admin_maintenance_runs', 'C'),
    ('ai_resolver_budget', 'P'),
    ('ai_resolver_cache', 'P'),
    ('album_artists', 'C'),
    ('album_tracks', 'C'),
    ('albums', 'C'),
    ('artist_attribution_revalidation', 'C'),
    ('artist_colike', 'C'),
    ('artist_coplay', 'C'),
    ('artist_sc_accounts', 'C'),
    ('artist_socials', 'C'),
    ('artists', 'C'),
    ('audio_index_wire_state', 'P'),
    ('background_job_enqueues', 'P'),
    ('background_job_failures', 'C'),
    ('background_jobs', 'P'),
    ('background_schedules', 'P'),
    ('catalog_audience', 'C'),
    ('catalog_collection_cursors', 'P'),
    ('catalog_collection_seen', 'P'),
    ('catalog_collection_sync', 'C'),
    ('catalog_merge_state', 'P'),
    ('catalog_work_links', 'P'),
    ('cdn_tracks', 'A'),
    ('cluster_bandit_stats', 'L'),
    ('cmd_dead', 'P'),
    ('cmd_done', 'X'),
    ('cmd_outbox', 'X'),
    ('cmd_pending', 'L'),
    ('cmd_purge', 'X'),
    ('cmd_reassert', 'X'),
    ('diag_bundles', 'A'),
    ('diag_events', 'A'),
    ('diag_issue_days', 'A'),
    ('diag_issue_origins', 'A'),
    ('discover_promoted', 'A'),
    ('discover_settings', 'A'),
    ('discover_tag_counts', 'C'),
    ('disliked_tracks', 'A'),
    ('enrich_calibration', 'P'),
    ('entity_reads', 'K'),
    ('featured_items', 'A'),
    ('job_status', 'C'),
    ('link_requests', 'A'),
    ('listening_history', 'A'),
    ('login_requests', 'A'),
    ('lyrics_cache', 'C'),
    ('lyrics_embedding_wire_state', 'P'),
    ('lyrics_lookup_backfill_state', 'P'),
    ('lyrics_lookup_state', 'C'),
    ('oauth_app_request_cooldowns', 'A'),
    ('oauth_app_token_issuance_reservations', 'P'),
    ('oauth_app_token_refresh_state', 'P'),
    ('oauth_app_tokens', 'C'),
    ('oauth_app_tokens_local', 'L'),
    ('oauth_apps', 'A'),
    ('pipeline_event_receipts', 'P'),
    ('playlist_legacy_membership_intents', 'C'),
    ('playlist_membership_operations', 'C'),
    ('playlist_membership_state', 'C'),
    ('playlist_reconcile_runs', 'P'),
    ('playlist_remote_observations', 'C'),
    ('playlist_remote_snapshot_tracks', 'P'),
    ('playlist_remote_snapshots', 'C'),
    ('playlist_sweep_cursor', 'P'),
    ('playlist_track_projection', 'C'),
    ('playlists', 'C'),
    ('recommendation_quality_models', 'P'),
    ('repl_checksums', 'K'),
    ('repl_purges', 'X'),
    ('repl_repair', 'X'),
    ('repl_resend', 'L'),
    ('repl_tombstones', 'A'),
    ('sc_egress_health', 'A'),
    ('sc_track_counters', 'K'),
    ('scd_unpublished_columns', 'L'),
    ('search_terms', 'C'),
    ('sessions', 'A'),
    ('soundcloud_connections', 'M'),
    ('star_track_tiers', 'P'),
    ('storage_event_repairs', 'P'),
    ('storage_event_state', 'P'),
    ('subscription_snapshot_state', 'P'),
    ('subscriptions', 'A'),
    ('sync_queue', 'M'),
    ('sync_queue_action_pauses', 'C'),
    ('taste_model_versions', 'C'),
    ('taste_schedule', 'P'),
    ('track_artist_blocks', 'A'),
    ('track_artists', 'C'),
    ('track_comments', 'C'),
    ('track_work_aliases', 'P'),
    ('tracks', 'C'),
    ('transcription_sync_versions', 'P'),
    ('transcription_wire_state', 'P'),
    ('user_auras', 'A'),
    ('user_blocked_artists', 'A'),
    ('user_collection_sync', 'P'),
    ('user_events', 'A'),
    ('user_followers', 'C'),
    ('user_followings', 'M'),
    ('user_like_ranks', 'A'),
    ('user_likes_playlists', 'M'),
    ('user_likes_tracks', 'M'),
    ('user_owned_playlists', 'C'),
    ('user_owned_tracks', 'C'),
    ('user_profiles', 'C'),
    ('user_superlike_ranks', 'A'),
    ('user_superlikes', 'A'),
    ('user_taste_vectors', 'C'),
    ('user_web_profiles', 'C'),
    ('users', 'C'),
    ('wanted_track_albums', 'C'),
    ('wanted_track_artists', 'C'),
    ('wanted_track_work_aliases', 'P'),
    ('wanted_tracks', 'C')
) AS v (tbl, cls) WHERE v.tbl = $1
$f$;

CREATE OR REPLACE FUNCTION public.scd_core_columns()
RETURNS TABLE (tbl text, col text)
LANGUAGE sql STABLE
AS $f$
VALUES
    ('artists', 'has_sc_account'),
    ('lyrics_cache', 'fts'),
    ('playlists', 'last_read_at')
$f$;

CREATE OR REPLACE FUNCTION public.scd_reassert_always(tbl text)
RETURNS boolean
LANGUAGE sql STABLE
AS $f$
SELECT $1 = ANY (ARRAY['artist_sc_accounts', 'artists', 'playlist_membership_operations', 'playlist_track_projection', 'track_comments', 'wanted_tracks']::text[])
$f$;

CREATE OR REPLACE FUNCTION public.scd_resend(tbl text)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    CASE tbl
        WHEN 'admin_maintenance_runs' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'admin_maintenance_runs', jsonb_build_object('kind', t.kind) FROM public.admin_maintenance_runs t ON CONFLICT DO NOTHING;
        WHEN 'album_artists' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'album_artists', jsonb_build_object('album_id', t.album_id, 'artist_id', t.artist_id, 'role', t.role) FROM public.album_artists t ON CONFLICT DO NOTHING;
        WHEN 'album_tracks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'album_tracks', jsonb_build_object('album_id', t.album_id, 'track_id', t.track_id) FROM public.album_tracks t ON CONFLICT DO NOTHING;
        WHEN 'albums' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'albums', jsonb_build_object('id', t.id) FROM public.albums t ON CONFLICT DO NOTHING;
        WHEN 'artist_attribution_revalidation' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'artist_attribution_revalidation', jsonb_build_object('singleton', t.singleton) FROM public.artist_attribution_revalidation t ON CONFLICT DO NOTHING;
        WHEN 'artist_colike' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'artist_colike', jsonb_build_object('a_id', t.a_id, 'b_id', t.b_id) FROM public.artist_colike t ON CONFLICT DO NOTHING;
        WHEN 'artist_coplay' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'artist_coplay', jsonb_build_object('a_id', t.a_id, 'b_id', t.b_id) FROM public.artist_coplay t ON CONFLICT DO NOTHING;
        WHEN 'artist_sc_accounts' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'artist_sc_accounts', jsonb_build_object('artist_id', t.artist_id, 'sc_user_id', t.sc_user_id) FROM public.artist_sc_accounts t ON CONFLICT DO NOTHING;
        WHEN 'artist_socials' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'artist_socials', jsonb_build_object('artist_id', t.artist_id, 'url', t.url) FROM public.artist_socials t ON CONFLICT DO NOTHING;
        WHEN 'artists' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'artists', jsonb_build_object('id', t.id) FROM public.artists t ON CONFLICT DO NOTHING;
        WHEN 'background_job_failures' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'background_job_failures', jsonb_build_object('id', t.id, 'generation', t.generation) FROM public.background_job_failures t ON CONFLICT DO NOTHING;
        WHEN 'catalog_audience' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'catalog_audience', jsonb_build_object('subject_urn', t.subject_urn, 'relation', t.relation, 'user_urn', t.user_urn) FROM public.catalog_audience t ON CONFLICT DO NOTHING;
        WHEN 'catalog_collection_sync' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'catalog_collection_sync', jsonb_build_object('subject_id', t.subject_id, 'collection', t.collection, 'scope', t.scope) FROM public.catalog_collection_sync t ON CONFLICT DO NOTHING;
        WHEN 'cdn_tracks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'cdn_tracks', jsonb_build_object('id', t.id) FROM public.cdn_tracks t ON CONFLICT DO NOTHING;
        WHEN 'diag_bundles' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'diag_bundles', jsonb_build_object('code', t.code) FROM public.diag_bundles t ON CONFLICT DO NOTHING;
        WHEN 'diag_events' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'diag_events', jsonb_build_object('id', t.id) FROM public.diag_events t ON CONFLICT DO NOTHING;
        WHEN 'diag_issue_days' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'diag_issue_days', jsonb_build_object('fingerprint', t.fingerprint, 'day', t.day, 'host', t.host, 'app_version', t.app_version) FROM public.diag_issue_days t ON CONFLICT DO NOTHING;
        WHEN 'diag_issue_origins' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'diag_issue_origins', jsonb_build_object('fingerprint', t.fingerprint, 'day', t.day, 'host', t.host, 'origin', t.origin) FROM public.diag_issue_origins t ON CONFLICT DO NOTHING;
        WHEN 'discover_promoted' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'discover_promoted', jsonb_build_object('id', t.id) FROM public.discover_promoted t ON CONFLICT DO NOTHING;
        WHEN 'discover_settings' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'discover_settings', jsonb_build_object('id', t.id) FROM public.discover_settings t ON CONFLICT DO NOTHING;
        WHEN 'discover_tag_counts' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'discover_tag_counts', jsonb_build_object('tag', t.tag) FROM public.discover_tag_counts t ON CONFLICT DO NOTHING;
        WHEN 'disliked_tracks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'disliked_tracks', jsonb_build_object('id', t.id) FROM public.disliked_tracks t ON CONFLICT DO NOTHING;
        WHEN 'entity_reads' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'entity_reads', jsonb_build_object('entity_urn', t.entity_urn) FROM public.entity_reads t ON CONFLICT DO NOTHING;
        WHEN 'featured_items' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'featured_items', jsonb_build_object('id', t.id) FROM public.featured_items t ON CONFLICT DO NOTHING;
        WHEN 'job_status' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'job_status', jsonb_build_object('kind', t.kind, 'dedup_key', t.dedup_key) FROM public.job_status t ON CONFLICT DO NOTHING;
        WHEN 'link_requests' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'link_requests', jsonb_build_object('id', t.id) FROM public.link_requests t ON CONFLICT DO NOTHING;
        WHEN 'listening_history' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'listening_history', jsonb_build_object('id', t.id) FROM public.listening_history t ON CONFLICT DO NOTHING;
        WHEN 'login_requests' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'login_requests', jsonb_build_object('id', t.id) FROM public.login_requests t ON CONFLICT DO NOTHING;
        WHEN 'lyrics_cache' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'lyrics_cache', jsonb_build_object('sc_track_id', t.sc_track_id) FROM public.lyrics_cache t ON CONFLICT DO NOTHING;
        WHEN 'lyrics_lookup_state' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'lyrics_lookup_state', jsonb_build_object('track_id', t.track_id) FROM public.lyrics_lookup_state t ON CONFLICT DO NOTHING;
        WHEN 'oauth_app_request_cooldowns' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'oauth_app_request_cooldowns', jsonb_build_object('oauth_app_id', t.oauth_app_id) FROM public.oauth_app_request_cooldowns t ON CONFLICT DO NOTHING;
        WHEN 'oauth_app_tokens' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'oauth_app_tokens', jsonb_build_object('oauth_app_id', t.oauth_app_id) FROM public.oauth_app_tokens t ON CONFLICT DO NOTHING;
        WHEN 'oauth_apps' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'oauth_apps', jsonb_build_object('id', t.id) FROM public.oauth_apps t ON CONFLICT DO NOTHING;
        WHEN 'playlist_legacy_membership_intents' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'playlist_legacy_membership_intents', jsonb_build_object('archive_id', t.archive_id) FROM public.playlist_legacy_membership_intents t ON CONFLICT DO NOTHING;
        WHEN 'playlist_membership_operations' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'playlist_membership_operations', jsonb_build_object('operation_id', t.operation_id) FROM public.playlist_membership_operations t ON CONFLICT DO NOTHING;
        WHEN 'playlist_membership_state' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'playlist_membership_state', jsonb_build_object('playlist_urn', t.playlist_urn) FROM public.playlist_membership_state t ON CONFLICT DO NOTHING;
        WHEN 'playlist_remote_observations' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'playlist_remote_observations', jsonb_build_object('id', t.id) FROM public.playlist_remote_observations t ON CONFLICT DO NOTHING;
        WHEN 'playlist_remote_snapshots' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'playlist_remote_snapshots', jsonb_build_object('id', t.id) FROM public.playlist_remote_snapshots t ON CONFLICT DO NOTHING;
        WHEN 'playlist_track_projection' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'playlist_track_projection', jsonb_build_object('playlist_urn', t.playlist_urn, 'position', t."position") FROM public.playlist_track_projection t ON CONFLICT DO NOTHING;
        WHEN 'playlists' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'playlists', jsonb_build_object('urn', t.urn) FROM public.playlists t ON CONFLICT DO NOTHING;
        WHEN 'repl_checksums' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'repl_checksums', jsonb_build_object('node', t.node, 'tbl', t.tbl, 'range', t.range) FROM public.repl_checksums t ON CONFLICT DO NOTHING;
        WHEN 'repl_tombstones' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'repl_tombstones', jsonb_build_object('tbl', t.tbl, 'pk', t.pk, 'v', t.v) FROM public.repl_tombstones t ON CONFLICT DO NOTHING;
        WHEN 'sc_egress_health' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'sc_egress_health', jsonb_build_object('channel', t.channel) FROM public.sc_egress_health t ON CONFLICT DO NOTHING;
        WHEN 'sc_track_counters' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'sc_track_counters', jsonb_build_object('sc_track_id', t.sc_track_id) FROM public.sc_track_counters t ON CONFLICT DO NOTHING;
        WHEN 'search_terms' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'search_terms', jsonb_build_object('word', t.word) FROM public.search_terms t ON CONFLICT DO NOTHING;
        WHEN 'sessions' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'sessions', jsonb_build_object('id', t.id) FROM public.sessions t ON CONFLICT DO NOTHING;
        WHEN 'soundcloud_connections' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'soundcloud_connections', jsonb_build_object('id', t.id) FROM public.soundcloud_connections t ON CONFLICT DO NOTHING;
        WHEN 'subscriptions' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'subscriptions', jsonb_build_object('user_urn', t.user_urn) FROM public.subscriptions t ON CONFLICT DO NOTHING;
        WHEN 'sync_queue' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'sync_queue', jsonb_build_object('id', t.id) FROM public.sync_queue t ON CONFLICT DO NOTHING;
        WHEN 'sync_queue_action_pauses' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'sync_queue_action_pauses', jsonb_build_object('oauth_app_id', t.oauth_app_id, 'action_type', t.action_type) FROM public.sync_queue_action_pauses t ON CONFLICT DO NOTHING;
        WHEN 'taste_model_versions' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'taste_model_versions', jsonb_build_object('version', t.version) FROM public.taste_model_versions t ON CONFLICT DO NOTHING;
        WHEN 'track_artist_blocks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'track_artist_blocks', jsonb_build_object('track_id', t.track_id, 'artist_id', t.artist_id) FROM public.track_artist_blocks t ON CONFLICT DO NOTHING;
        WHEN 'track_artists' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'track_artists', jsonb_build_object('track_id', t.track_id, 'artist_id', t.artist_id, 'role', t.role) FROM public.track_artists t ON CONFLICT DO NOTHING;
        WHEN 'track_comments' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'track_comments', jsonb_build_object('id', t.id) FROM public.track_comments t ON CONFLICT DO NOTHING;
        WHEN 'tracks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'tracks', jsonb_build_object('id', t.id) FROM public.tracks t ON CONFLICT DO NOTHING;
        WHEN 'user_auras' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_auras', jsonb_build_object('user_urn', t.user_urn) FROM public.user_auras t ON CONFLICT DO NOTHING;
        WHEN 'user_blocked_artists' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_blocked_artists', jsonb_build_object('sc_user_id', t.sc_user_id, 'kind', t.kind, 'target_id', t.target_id) FROM public.user_blocked_artists t ON CONFLICT DO NOTHING;
        WHEN 'user_events' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_events', jsonb_build_object('id', t.id) FROM public.user_events t ON CONFLICT DO NOTHING;
        WHEN 'user_followers' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_followers', jsonb_build_object('user_id', t.user_id, 'target_user_urn', t.target_user_urn) FROM public.user_followers t ON CONFLICT DO NOTHING;
        WHEN 'user_followings' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_followings', jsonb_build_object('user_id', t.user_id, 'target_user_urn', t.target_user_urn) FROM public.user_followings t ON CONFLICT DO NOTHING;
        WHEN 'user_like_ranks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_like_ranks', jsonb_build_object('user_id', t.user_id, 'sc_track_id', t.sc_track_id) FROM public.user_like_ranks t ON CONFLICT DO NOTHING;
        WHEN 'user_likes_playlists' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_likes_playlists', jsonb_build_object('user_id', t.user_id, 'playlist_urn', t.playlist_urn) FROM public.user_likes_playlists t ON CONFLICT DO NOTHING;
        WHEN 'user_likes_tracks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_likes_tracks', jsonb_build_object('user_id', t.user_id, 'sc_track_id', t.sc_track_id) FROM public.user_likes_tracks t ON CONFLICT DO NOTHING;
        WHEN 'user_owned_playlists' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_owned_playlists', jsonb_build_object('user_id', t.user_id, 'playlist_urn', t.playlist_urn) FROM public.user_owned_playlists t ON CONFLICT DO NOTHING;
        WHEN 'user_owned_tracks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_owned_tracks', jsonb_build_object('user_id', t.user_id, 'sc_track_id', t.sc_track_id) FROM public.user_owned_tracks t ON CONFLICT DO NOTHING;
        WHEN 'user_profiles' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_profiles', jsonb_build_object('soundcloud_user_id', t.soundcloud_user_id) FROM public.user_profiles t ON CONFLICT DO NOTHING;
        WHEN 'user_superlike_ranks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_superlike_ranks', jsonb_build_object('user_id', t.user_id, 'sc_track_id', t.sc_track_id) FROM public.user_superlike_ranks t ON CONFLICT DO NOTHING;
        WHEN 'user_superlikes' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_superlikes', jsonb_build_object('user_id', t.user_id, 'sc_track_id', t.sc_track_id) FROM public.user_superlikes t ON CONFLICT DO NOTHING;
        WHEN 'user_taste_vectors' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_taste_vectors', jsonb_build_object('sc_user_id', t.sc_user_id, 'version', t.version) FROM public.user_taste_vectors t ON CONFLICT DO NOTHING;
        WHEN 'user_web_profiles' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_web_profiles', jsonb_build_object('sc_user_id', t.sc_user_id) FROM public.user_web_profiles t ON CONFLICT DO NOTHING;
        WHEN 'users' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'users', jsonb_build_object('sc_user_id', t.sc_user_id) FROM public.users t ON CONFLICT DO NOTHING;
        WHEN 'wanted_track_albums' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'wanted_track_albums', jsonb_build_object('wanted_track_id', t.wanted_track_id, 'album_id', t.album_id) FROM public.wanted_track_albums t ON CONFLICT DO NOTHING;
        WHEN 'wanted_track_artists' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'wanted_track_artists', jsonb_build_object('wanted_track_id', t.wanted_track_id, 'artist_id', t.artist_id, 'role', t.role) FROM public.wanted_track_artists t ON CONFLICT DO NOTHING;
        WHEN 'wanted_tracks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'wanted_tracks', jsonb_build_object('id', t.id) FROM public.wanted_tracks t ON CONFLICT DO NOTHING;
        ELSE
            RAISE EXCEPTION 'public.scd_resend: unknown table %', tbl;
    END CASE;
    RETURN;
END
$f$;
CREATE OR REPLACE FUNCTION public.scd_resend_all()
RETURNS bigint
LANGUAGE plpgsql
AS $f$
DECLARE
    sent text[] := CASE public.scd_role() WHEN 'core' THEN ARRAY['C', 'M', 'A'] ELSE ARRAY['M', 'A', 'K'] END;
    r record;
BEGIN
    FOR r IN SELECT v.tbl FROM (VALUES
        ('admin_maintenance_runs', 'C'),
        ('album_artists', 'C'),
        ('album_tracks', 'C'),
        ('albums', 'C'),
        ('artist_attribution_revalidation', 'C'),
        ('artist_coplay', 'C'),
        ('artist_sc_accounts', 'C'),
        ('artist_socials', 'C'),
        ('artists', 'C'),
        ('background_job_failures', 'C'),
        ('catalog_audience', 'C'),
        ('catalog_collection_sync', 'C'),
        ('cdn_tracks', 'A'),
        ('diag_bundles', 'A'),
        ('diag_events', 'A'),
        ('diag_issue_days', 'A'),
        ('diag_issue_origins', 'A'),
        ('discover_promoted', 'A'),
        ('discover_settings', 'A'),
        ('discover_tag_counts', 'C'),
        ('disliked_tracks', 'A'),
        ('entity_reads', 'K'),
        ('featured_items', 'A'),
        ('job_status', 'C'),
        ('link_requests', 'A'),
        ('listening_history', 'A'),
        ('login_requests', 'A'),
        ('lyrics_cache', 'C'),
        ('lyrics_lookup_state', 'C'),
        ('oauth_app_request_cooldowns', 'A'),
        ('oauth_app_tokens', 'C'),
        ('oauth_apps', 'A'),
        ('playlist_legacy_membership_intents', 'C'),
        ('playlist_membership_operations', 'C'),
        ('playlist_membership_state', 'C'),
        ('playlist_remote_observations', 'C'),
        ('playlist_remote_snapshots', 'C'),
        ('playlist_track_projection', 'C'),
        ('playlists', 'C'),
        ('repl_checksums', 'K'),
        ('repl_tombstones', 'A'),
        ('sc_egress_health', 'A'),
        ('sc_track_counters', 'K'),
        ('search_terms', 'C'),
        ('sessions', 'A'),
        ('soundcloud_connections', 'M'),
        ('subscriptions', 'A'),
        ('sync_queue', 'M'),
        ('sync_queue_action_pauses', 'C'),
        ('taste_model_versions', 'C'),
        ('track_artist_blocks', 'A'),
        ('track_artists', 'C'),
        ('track_comments', 'C'),
        ('tracks', 'C'),
        ('user_auras', 'A'),
        ('user_blocked_artists', 'A'),
        ('user_events', 'A'),
        ('user_followers', 'C'),
        ('user_followings', 'M'),
        ('user_like_ranks', 'A'),
        ('user_likes_playlists', 'M'),
        ('user_likes_tracks', 'M'),
        ('user_owned_playlists', 'C'),
        ('user_owned_tracks', 'C'),
        ('user_profiles', 'C'),
        ('user_superlike_ranks', 'A'),
        ('user_superlikes', 'A'),
        ('user_taste_vectors', 'C'),
        ('user_web_profiles', 'C'),
        ('users', 'C'),
        ('wanted_track_albums', 'C'),
        ('wanted_track_artists', 'C'),
        ('wanted_tracks', 'C')
    ) AS v (tbl, cls) WHERE v.cls = ANY (sent) ORDER BY v.tbl LOOP
        PERFORM public.scd_resend(r.tbl);
    END LOOP;
    RETURN (SELECT count(*) FROM public.repl_resend);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_publication_lists()
RETURNS TABLE (pubname text, node text, publish text, tbl text, cols text[])
LANGUAGE sql IMMUTABLE
AS $f$
VALUES
    ('scd_api_pub', 'api', 'insert, update, delete', 'cdn_tracks', ARRAY['cdn_path', 'created_at', 'id', 'quality', 'row_v', 'status', 'stored_quality', 'track_urn', 'updated_at']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'diag_bundles', ARRAY['app_version', 'at', 'body', 'code', 'os']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'diag_events', ARRAY['app_version', 'at', 'body', 'fingerprint', 'id', 'install', 'kind', 'os', 'title', 'trace']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'diag_issue_days', ARRAY['app_version', 'day', 'events', 'fingerprint', 'first_at', 'host', 'kind', 'last_at', 'row_v', 'title']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'diag_issue_origins', ARRAY['day', 'fingerprint', 'host', 'origin']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'discover_promoted', ARRAY['active', 'created_at', 'entity_id', 'entity_type', 'id', 'note', 'position', 'row_v', 'updated_at']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'discover_settings', ARRAY['id', 'row_v', 'show_star', 'star_limit', 'star_strategy', 'updated_at']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'disliked_tracks', ARRAY['created_at', 'id', 'row_v', 'sc_track_id', 'sc_user_id', 'track_data']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'entity_reads', ARRAY['entity_urn', 'last_read_at']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'featured_items', ARRAY['active', 'created_at', 'id', 'row_v', 'sc_urn', 'type', 'weight']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'link_requests', ARRAY['claim_token', 'created_at', 'error', 'expires_at', 'id', 'mode', 'row_v', 'source_session_id', 'status', 'target_session_id']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'listening_history', ARRAY['artist_name', 'artist_urn', 'artwork_url', 'duration', 'id', 'played_at', 'sc_track_id', 'soundcloud_user_id', 'title']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'login_requests', ARRAY['code_verifier', 'created_at', 'error', 'expires_at', 'id', 'oauth_app_id', 'profile_ok', 'redirect_url', 'result_session_id', 'retry_count', 'row_v', 'state', 'status', 'step', 'target_session_id', 'username']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'oauth_app_request_cooldowns', ARRAY['failure_count', 'oauth_app_id', 'retry_at', 'row_v', 'updated_at']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'oauth_apps', ARRAY['active', 'client_id', 'client_secret', 'created_at', 'id', 'name', 'redirect_uri', 'row_v', 'updated_at']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'repl_checksums', ARRAY['at', 'hash', 'node', 'range', 'tbl']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'repl_repair', ARRAY['at', 'id', 'pk', 'row', 'src_node', 'tbl']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'repl_tombstones', ARRAY['at', 'pk', 'tbl', 'v']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'sc_egress_health', ARRAY['channel', 'open_until', 'opened_at', 'opened_by', 'row_v']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'sc_track_counters', ARRAY['comment_count', 'fetched_at', 'likes_count', 'play_count', 'reposts_count', 'sc_track_id']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'sessions', ARRAY['created_at', 'id', 'row_v', 'soundcloud_connection_id', 'updated_at']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'soundcloud_connections', ARRAY['access_token', 'created_at', 'expires_at', 'id', 'oauth_app_id', 'refresh_token', 'scope', 'soundcloud_user_id', 'token_v', 'username']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'subscriptions', ARRAY['exp_date', 'row_v', 'user_urn']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'sync_queue', ARRAY['action_type', 'cancelled', 'created_at', 'generation', 'id', 'intent_v', 'payload', 'target_urn', 'user_id']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'track_artist_blocks', ARRAY['artist_id', 'created_at', 'note', 'row_v', 'track_id']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'user_auras', ARRAY['aura_id', 'custom_hex', 'row_v', 'updated_at', 'user_urn']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'user_blocked_artists', ARRAY['avatar_url', 'created_at', 'kind', 'name', 'row_v', 'sc_user_id', 'sc_user_ids', 'target_id']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'user_events', ARRAY['created_at', 'event_type', 'id', 'position_pct', 'sc_track_id', 'sc_user_id', 'weight']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'user_followings', ARRAY['created_at', 'target_user_urn', 'user_id', 'wanted_state', 'wanted_v']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'user_like_ranks', ARRAY['like_created_at', 'rank', 'row_v', 'sc_track_id', 'user_id']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'user_likes_playlists', ARRAY['created_at', 'playlist_urn', 'user_id', 'wanted_state', 'wanted_v']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'user_likes_tracks', ARRAY['created_at', 'sc_track_id', 'user_id', 'wanted_state', 'wanted_v']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'user_superlike_ranks', ARRAY['rank', 'row_v', 'sc_track_id', 'superlike_created_at', 'user_id']::text[]),
    ('scd_api_pub', 'api', 'insert, update, delete', 'user_superlikes', ARRAY['created_at', 'like_created_at', 'row_v', 'sc_track_id', 'user_id']::text[]),
    ('scd_cmd_pub', 'api', 'insert', 'cmd_outbox', ARRAY['args', 'created_at', 'id', 'kind', 'node', 'touched']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'admin_maintenance_runs', ARRAY['changed', 'completed_at', 'cursor_text', 'cursor_uuid', 'kind', 'merged', 'phase', 'run_id', 'scanned', 'skipped', 'started_at', 'status', 'updated_at']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'album_artists', ARRAY['album_id', 'artist_id', 'role']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'album_tracks', ARRAY['album_id', 'disc_number', 'position', 'track_id']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'albums', ARRAY['aggregates_updated_at', 'confidence', 'cover_url', 'created_at', 'genius_album_id', 'id', 'is_star_artist', 'mb_release_id', 'normalized_title', 'popularity_score', 'primary_artist_id', 'release_date', 'release_year', 'source', 'spotify_album_id', 'title', 'total_duration_ms', 'track_count', 'type', 'updated_at']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'artist_attribution_revalidation', ARRAY['credits_removed', 'identity_completed_at', 'identity_cursor_account', 'identity_cursor_artist', 'singleton', 'tracks_reset', 'updated_at', 'walker_completed_at']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'artist_coplay', ARRAY['a_id', 'b_id', 'last_seen', 'weight']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'artist_sc_accounts', ARRAY['artist_id', 'created_at', 'notes', 'role', 'sc_user_id', 'source', 'updated_at', 'verified']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'artist_socials', ARRAY['artist_id', 'created_at', 'kind', 'source', 'updated_at', 'url', 'verified']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'artists', ARRAY['account_walk_locked_at', 'aggregates_updated_at', 'album_count_denorm', 'avatar_url', 'bio', 'confidence', 'country', 'crawl_attempts', 'crawl_dead', 'crawl_fail_count', 'created_at', 'genius_artist_id', 'genius_crawl_offset', 'genius_crawled_at', 'genius_locked_at', 'genius_next_run_at', 'has_sc_account', 'id', 'interest_score', 'is_star', 'isni', 'last_account_walk_at', 'last_crawled_at', 'mb_artist_id', 'mb_crawl_offset', 'mb_crawled_at', 'mb_locked_at', 'mb_next_run_at', 'merged_into', 'monthly_listeners', 'name', 'normalized_name', 'popularity_score', 'sc_user_id', 'source', 'spotify_artist_id', 'star_aura_id', 'star_custom_hex', 'tags', 'track_count_featured', 'track_count_primary', 'trending_score', 'updated_at']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'background_job_failures', ARRAY['attempts', 'created_at', 'dedup_key', 'failed_at', 'generation', 'id', 'kind', 'lane', 'last_error', 'max_attempts', 'payload', 'priority']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'catalog_audience', ARRAY['created_at', 'relation', 'subject_urn', 'synced_at', 'user_urn']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'catalog_collection_sync', ARRAY['collection', 'complete', 'generation', 'item_count', 'job_id', 'next_cursor', 'page_count', 'scope', 'snapshot_id', 'started_at', 'subject_id', 'synced_at', 'updated_at', 'verified_at']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'cdn_tracks', ARRAY['cdn_path', 'created_at', 'id', 'quality', 'row_v', 'status', 'stored_quality', 'track_urn', 'updated_at']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'cmd_done', ARRAY['at', 'id', 'node', 'status']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'cmd_purge', ARRAY['id', 'node', 'pk', 'tbl']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'cmd_reassert', ARRAY['id', 'node', 'pk', 'row', 'tbl']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'diag_bundles', ARRAY['app_version', 'at', 'body', 'code', 'os']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'diag_events', ARRAY['app_version', 'at', 'body', 'fingerprint', 'id', 'install', 'kind', 'os', 'title', 'trace']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'diag_issue_days', ARRAY['app_version', 'day', 'events', 'fingerprint', 'first_at', 'host', 'kind', 'last_at', 'row_v', 'title']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'diag_issue_origins', ARRAY['day', 'fingerprint', 'host', 'origin']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'discover_promoted', ARRAY['active', 'created_at', 'entity_id', 'entity_type', 'id', 'note', 'position', 'row_v', 'updated_at']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'discover_settings', ARRAY['id', 'row_v', 'show_star', 'star_limit', 'star_strategy', 'updated_at']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'discover_tag_counts', ARRAY['artist_count', 'refreshed_at', 'tag']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'disliked_tracks', ARRAY['created_at', 'id', 'row_v', 'sc_track_id', 'sc_user_id', 'track_data']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'featured_items', ARRAY['active', 'created_at', 'id', 'row_v', 'sc_urn', 'type', 'weight']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'job_status', ARRAY['available_at', 'dedup_key', 'kind', 'state', 'updated_at']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'link_requests', ARRAY['claim_token', 'created_at', 'error', 'expires_at', 'id', 'mode', 'row_v', 'source_session_id', 'status', 'target_session_id']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'listening_history', ARRAY['artist_name', 'artist_urn', 'artwork_url', 'duration', 'id', 'played_at', 'sc_track_id', 'soundcloud_user_id', 'title']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'login_requests', ARRAY['code_verifier', 'created_at', 'error', 'expires_at', 'id', 'oauth_app_id', 'profile_ok', 'redirect_url', 'result_session_id', 'retry_count', 'row_v', 'state', 'status', 'step', 'target_session_id', 'username']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'lyrics_cache', ARRAY['content_generation', 'content_hash', 'created_at', 'embedded_at', 'embedding_state', 'fts', 'language', 'language_confidence', 'plain_source', 'plain_text', 'sc_track_id', 'source', 'synced_lrc', 'synced_source', 'synced_version', 'track_id', 'updated_at']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'lyrics_lookup_state', ARRAY['algorithm_version', 'attempts', 'claim_expires_at', 'claim_job_generation', 'claim_job_id', 'claim_job_lease_id', 'claim_state_generation', 'created_at', 'failure_streak', 'generation', 'input_artist', 'input_duration_ms', 'input_genius_song_id', 'input_genius_url', 'input_release_date', 'input_sc_created_at', 'input_title', 'last_attempt_at', 'last_error', 'last_outcome', 'miss_streak', 'next_run_at', 'priority', 'retry_after_at', 'sc_track_id', 'source_set_version', 'status', 'track_id', 'updated_at', 'wake_durable_at', 'wake_generation', 'wake_message_id']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'oauth_app_request_cooldowns', ARRAY['failure_count', 'oauth_app_id', 'retry_at', 'row_v', 'updated_at']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'oauth_app_tokens', ARRAY['access_token', 'expires_at', 'generation', 'last_refresh_error', 'last_used_at', 'oauth_app_id', 'refresh_attempts', 'refresh_token', 'refreshed_at', 'scope']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'oauth_apps', ARRAY['active', 'client_id', 'client_secret', 'created_at', 'id', 'name', 'redirect_uri', 'row_v', 'updated_at']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'playlist_legacy_membership_intents', ARRAY['archive_id', 'archived_at', 'classification', 'legacy_desired_revision', 'legacy_synced_revision', 'legacy_user_id', 'local_snapshot_id', 'playlist_urn', 'prior_classification', 'queue_created_at', 'queue_failed_at', 'queue_generation', 'queue_id', 'queue_last_error', 'queue_next_run_at', 'queue_payload', 'queue_retry_count', 'remote_attempted_generation', 'remote_completed_generation', 'remote_result', 'resolved_at', 'source']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'playlist_membership_operations', ARRAY['accepted_projection_revision', 'actor_sc_user_id', 'base_baseline_generation', 'base_observation_id', 'boundary', 'conflict_code', 'created_at', 'expected_projection_revision', 'idempotency_key', 'kind', 'left_anchor_track_id', 'operation_id', 'ordered_track_ids', 'outcome', 'playlist_urn', 'request_fingerprint', 'resolved_at', 'right_anchor_track_id', 'sequence', 'track_id']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'playlist_membership_state', ARRAY['baseline_generation', 'baseline_observation_id', 'candidate_fingerprint', 'committed_operation_sequence', 'conflict_code', 'last_error', 'last_operation_sequence', 'latest_observation_id', 'next_reconcile_at', 'owner_sc_user_id', 'playlist_urn', 'projection_revision', 'projection_track_count', 'reconcile_failure_streak', 'reconcile_generation', 'remote_applied_at', 'remote_apply_fingerprint', 'remote_apply_generation', 'sync_status', 'updated_at']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'playlist_remote_observations', ARRAY['all_items_identified', 'authority', 'declared_track_count', 'error_kind', 'id', 'observed_at', 'observed_track_count', 'outcome', 'pagination_complete', 'playlist_urn', 'retry_at', 'sc_last_modified', 'snapshot_id']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'playlist_remote_snapshots', ARRAY['content_fingerprint', 'created_at', 'id', 'playlist_urn', 'track_count']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'playlist_track_projection', ARRAY['playlist_urn', 'position', 'sc_track_id']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'playlists', ARRAY['artwork_url', 'created_at', 'deleted_at', 'description', 'duration_ms', 'genre', 'kind', 'label_name', 'last_read_at', 'likes_count_sc', 'owner_sc_user_id', 'owner_urn', 'owner_username', 'permalink_url', 'playlist_type', 'release_date', 'release_year', 'reposts_count_sc', 'sc_created_at', 'sc_desired', 'sc_last_modified', 'sc_metadata', 'sc_mutation_observation', 'sc_observation', 'sc_playlist_id', 'sc_synced_at', 'sc_write_confirmed', 'sharing', 'tags', 'title', 'title_normalized', 'track_count', 'updated_at', 'urn']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'repl_purges', ARRAY['at', 'pk', 'purged_v', 'tbl']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'repl_repair', ARRAY['at', 'id', 'pk', 'row', 'src_node', 'tbl']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'repl_tombstones', ARRAY['at', 'pk', 'tbl', 'v']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'sc_egress_health', ARRAY['channel', 'open_until', 'opened_at', 'opened_by', 'row_v']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'search_terms', ARRAY['latin', 'ndoc', 'word']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'sessions', ARRAY['created_at', 'id', 'row_v', 'soundcloud_connection_id', 'updated_at']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'soundcloud_connections', ARRAY['access_token', 'created_at', 'expires_at', 'first_refresh_rejection_at', 'id', 'last_refresh_attempt_at', 'last_refresh_error', 'last_refresh_error_kind', 'last_refresh_success_at', 'login_v', 'oauth_app_id', 'refresh_failure_count', 'refresh_generation', 'refresh_lease_expires_at', 'refresh_lease_id', 'refresh_rejection_count', 'refresh_token', 'retry_at', 'scope', 'soundcloud_user_id', 'token_v', 'updated_at', 'username']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'subscriptions', ARRAY['exp_date', 'row_v', 'user_urn']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'sync_queue', ARRAY['action_type', 'cancelled', 'created_at', 'dead', 'exec_generation', 'failed_at', 'generation', 'id', 'intent_v', 'last_error', 'lease_generation', 'lease_id', 'locked_at', 'next_run_at', 'payload', 'remote_attempted_generation', 'remote_completed_generation', 'remote_result', 'retry_count', 'target_urn', 'user_id']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'sync_queue_action_pauses', ARRAY['action_type', 'oauth_app_id', 'paused_until', 'reason', 'strikes', 'updated_at']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'taste_model_versions', ARRAY['active', 'applied_at', 'collection', 'created_at', 'dim', 'input_object', 'items_count', 'metrics', 'pooling', 'refreshed_through', 'trained_at', 'users_count', 'version']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'track_artist_blocks', ARRAY['artist_id', 'created_at', 'note', 'row_v', 'track_id']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'track_artists', ARRAY['artist_id', 'confidence', 'evidence', 'position', 'role', 'source', 'track_id']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'track_comments', ARRAY['body', 'created_at', 'id', 'sc_comment_id', 'sc_created_at', 'sc_track_id', 'synced_at', 'track_position_ms', 'user_urn']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'tracks', ARRAY['album_id', 'album_position', 'artwork_url', 'audio_fingerprint', 'canonical_track_id', 'comments_count_sc', 'cover_of_artist_id', 'created_at', 'deleted_at', 'description', 'duration_ms', 'duration_resolve_attempts', 'duration_resolve_retry_at', 'enrich_attempts', 'enrich_confidence', 'enrich_error', 'enrich_locked_at', 'enrich_next_run_at', 'enrich_source', 'enrich_state', 'enriched_at', 'genius_song_id', 'genius_url', 'genre', 'hq_upgrade_attempts', 'hq_upgrade_last_at', 'hq_upgrade_pending', 'id', 'index_attempts', 'index_priority', 'index_state', 'indexed_at', 'is_cover', 'isrc', 'language', 'language_confidence', 'likes_count_sc', 'metadata_artist', 'needs_duration_resolve', 'permalink_url', 'pipeline_held', 'play_count_sc', 'primary_artist_id', 'quality_model_version', 'quality_score', 'recording_key', 'release_date', 'release_year', 'reposts_count_sc', 's3_missing_at', 's3_verified_at', 'sc_created_at', 'sc_desired', 'sc_last_modified', 'sc_metadata', 'sc_mutation_observation', 'sc_observation', 'sc_synced_at', 'sc_track_id', 'sc_write_confirmed', 'sharing', 'storage_attempts', 'storage_priority', 'storage_quality', 'storage_state', 'superseded_by', 'tags', 'title', 'title_normalized', 'transcribe_at', 'transcribe_state', 'updated_at', 'upload_kind', 'uploader_avatar_url', 'uploader_sc_user_id', 'uploader_urn', 'uploader_username', 'urn', 'waveform_url', 'work_key', 'work_normalizer_version']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'user_auras', ARRAY['aura_id', 'custom_hex', 'row_v', 'updated_at', 'user_urn']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'user_blocked_artists', ARRAY['avatar_url', 'created_at', 'kind', 'name', 'row_v', 'sc_user_id', 'sc_user_ids', 'target_id']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'user_events', ARRAY['created_at', 'event_type', 'id', 'position_pct', 'sc_track_id', 'sc_user_id', 'weight']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'user_followers', ARRAY['created_at', 'progress', 'synced_at', 'target_user_urn', 'user_id']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'user_followings', ARRAY['created_at', 'progress', 'synced_at', 'synced_for_v', 'target_user_urn', 'user_id', 'wanted_state', 'wanted_v']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'user_like_ranks', ARRAY['like_created_at', 'rank', 'row_v', 'sc_track_id', 'user_id']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'user_likes_playlists', ARRAY['created_at', 'playlist_urn', 'progress', 'synced_at', 'synced_for_v', 'user_id', 'wanted_state', 'wanted_v']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'user_likes_tracks', ARRAY['created_at', 'liked_at', 'progress', 'sc_track_id', 'synced_at', 'synced_for_v', 'user_id', 'wanted_state', 'wanted_v']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'user_owned_playlists', ARRAY['created_at', 'playlist_urn', 'progress', 'synced_at', 'user_id']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'user_owned_tracks', ARRAY['created_at', 'progress', 'sc_track_id', 'synced_at', 'user_id']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'user_profiles', ARRAY['profile_json', 'sc_observation', 'soundcloud_user_id', 'synced_at']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'user_superlike_ranks', ARRAY['rank', 'row_v', 'sc_track_id', 'superlike_created_at', 'user_id']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'user_superlikes', ARRAY['created_at', 'like_created_at', 'row_v', 'sc_track_id', 'user_id']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'user_taste_vectors', ARRAY['sc_user_id', 'updated_at', 'vec', 'version']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'user_web_profiles', ARRAY['profiles', 'sc_observation', 'sc_user_id', 'synced_at']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'users', ARRAY['avatar_url', 'city', 'comments_count', 'country', 'created_at', 'description', 'first_name', 'followers_count', 'followings_count', 'full_name', 'kind', 'last_name', 'permalink', 'permalink_url', 'playlists_count', 'reposts_count', 'sc_created_at', 'sc_last_modified', 'sc_observation', 'sc_synced_at', 'sc_user_id', 'tracks_count', 'updated_at', 'urn', 'username', 'username_normalized', 'verified']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'wanted_track_albums', ARRAY['album_id', 'position', 'wanted_track_id']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'wanted_track_artists', ARRAY['artist_id', 'position', 'role', 'wanted_track_id']::text[]),
    ('scd_core_pub', 'core', 'insert, update, delete', 'wanted_tracks', ARRAY['discovered_at', 'duration_ms', 'external_id', 'fetched_at', 'id', 'isrc', 'normalized_title', 'primary_artist_id', 'recording_key', 'release_year', 'resolve_attempts', 'resolve_error', 'resolve_locked_at', 'resolve_next_run_at', 'source', 'status', 'title', 'track_id', 'updated_at', 'work_key', 'work_normalizer_version', 'work_reconciled_at']::text[])
$f$;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
CREATE OR REPLACE FUNCTION public.scd_row_hash_admin_catalog_requests(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.command, t.created_at, t.reply, t.request_id)::text) FROM public.admin_catalog_requests t WHERE t.request_id = (k ->> 'request_id')::uuid
$f$;

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

CREATE OR REPLACE FUNCTION public.scd_purge_admin_catalog_requests(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.request_id INTO p FROM public.admin_catalog_requests t WHERE t.request_id = (k ->> 'request_id')::uuid;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.admin_catalog_requests t WHERE t.request_id = (k ->> 'request_id')::uuid;
END
$f$;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.admin_catalog_requests FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_admin_catalog_requests', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.admin_maintenance_runs FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_admin_maintenance_runs', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_admin_maintenance_runs()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'admin_maintenance_runs' AND p.pk = jsonb_build_object('kind', OLD.kind)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.kind IS DISTINCT FROM OLD.kind THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'admin_maintenance_runs' AND p.pk = jsonb_build_object('kind', OLD.kind)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'admin_maintenance_runs' AND p.pk = jsonb_build_object('kind', NEW.kind)) THEN
                    INSERT INTO public.admin_maintenance_runs (changed, completed_at, cursor_text, cursor_uuid, kind, merged, phase, run_id, scanned, skipped, started_at, status, updated_at) VALUES (NEW.changed, NEW.completed_at, NEW.cursor_text, NEW.cursor_uuid, NEW.kind, NEW.merged, NEW.phase, NEW.run_id, NEW.scanned, NEW.skipped, NEW.started_at, NEW.status, NEW.updated_at);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'admin_maintenance_runs' AND p.pk = jsonb_build_object('kind', NEW.kind)) THEN
                DELETE FROM public.admin_maintenance_runs t WHERE t.kind = OLD.kind;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'admin_maintenance_runs' AND p.pk = jsonb_build_object('kind', NEW.kind)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.admin_maintenance_runs t WHERE t.kind = NEW.kind FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'admin_maintenance_runs' AND p.pk = jsonb_build_object('kind', NEW.kind)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.admin_maintenance_runs t SET changed = NEW.changed, completed_at = NEW.completed_at, cursor_text = NEW.cursor_text, cursor_uuid = NEW.cursor_uuid, merged = NEW.merged, phase = NEW.phase, run_id = NEW.run_id, scanned = NEW.scanned, skipped = NEW.skipped, started_at = NEW.started_at, status = NEW.status, updated_at = NEW.updated_at WHERE t.kind = NEW.kind;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.admin_maintenance_runs (changed, completed_at, cursor_text, cursor_uuid, kind, merged, phase, run_id, scanned, skipped, started_at, status, updated_at)
        VALUES (NEW.changed, NEW.completed_at, NEW.cursor_text, NEW.cursor_uuid, NEW.kind, NEW.merged, NEW.phase, NEW.run_id, NEW.scanned, NEW.skipped, NEW.started_at, NEW.status, NEW.updated_at)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.admin_maintenance_runs still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.admin_maintenance_runs FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_admin_maintenance_runs();
ALTER TABLE public.admin_maintenance_runs ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.admin_maintenance_runs FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.admin_maintenance_runs ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.ai_resolver_budget FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_ai_resolver_budget', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.ai_resolver_cache FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_ai_resolver_cache', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.album_artists FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_album_artists', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_album_artists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'album_artists' AND p.pk = jsonb_build_object('album_id', OLD.album_id, 'artist_id', OLD.artist_id, 'role', OLD.role)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.album_id, NEW.artist_id, NEW.role) IS DISTINCT FROM (OLD.album_id, OLD.artist_id, OLD.role) THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'album_artists' AND p.pk = jsonb_build_object('album_id', OLD.album_id, 'artist_id', OLD.artist_id, 'role', OLD.role)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'album_artists' AND p.pk = jsonb_build_object('album_id', NEW.album_id, 'artist_id', NEW.artist_id, 'role', NEW.role)) THEN
                    INSERT INTO public.album_artists (album_id, artist_id, role) VALUES (NEW.album_id, NEW.artist_id, NEW.role);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'album_artists' AND p.pk = jsonb_build_object('album_id', NEW.album_id, 'artist_id', NEW.artist_id, 'role', NEW.role)) THEN
                DELETE FROM public.album_artists t WHERE (t.album_id, t.artist_id, t.role) = (OLD.album_id, OLD.artist_id, OLD.role);
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'album_artists' AND p.pk = jsonb_build_object('album_id', NEW.album_id, 'artist_id', NEW.artist_id, 'role', NEW.role)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.album_artists t WHERE (t.album_id, t.artist_id, t.role) = (NEW.album_id, NEW.artist_id, NEW.role) FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'album_artists' AND p.pk = jsonb_build_object('album_id', NEW.album_id, 'artist_id', NEW.artist_id, 'role', NEW.role)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.album_artists (album_id, artist_id, role)
        VALUES (NEW.album_id, NEW.artist_id, NEW.role)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.album_artists still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.album_artists FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_album_artists();
ALTER TABLE public.album_artists ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.album_artists FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.album_artists ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.album_tracks FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_album_tracks', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_album_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'album_tracks' AND p.pk = jsonb_build_object('album_id', OLD.album_id, 'track_id', OLD.track_id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.album_id, NEW.track_id) IS DISTINCT FROM (OLD.album_id, OLD.track_id) THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'album_tracks' AND p.pk = jsonb_build_object('album_id', OLD.album_id, 'track_id', OLD.track_id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'album_tracks' AND p.pk = jsonb_build_object('album_id', NEW.album_id, 'track_id', NEW.track_id)) THEN
                    INSERT INTO public.album_tracks (album_id, disc_number, "position", track_id) VALUES (NEW.album_id, NEW.disc_number, NEW."position", NEW.track_id);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'album_tracks' AND p.pk = jsonb_build_object('album_id', NEW.album_id, 'track_id', NEW.track_id)) THEN
                DELETE FROM public.album_tracks t WHERE (t.album_id, t.track_id) = (OLD.album_id, OLD.track_id);
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'album_tracks' AND p.pk = jsonb_build_object('album_id', NEW.album_id, 'track_id', NEW.track_id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.album_tracks t WHERE (t.album_id, t.track_id) = (NEW.album_id, NEW.track_id) FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'album_tracks' AND p.pk = jsonb_build_object('album_id', NEW.album_id, 'track_id', NEW.track_id)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.album_tracks t SET disc_number = NEW.disc_number, "position" = NEW."position" WHERE (t.album_id, t.track_id) = (NEW.album_id, NEW.track_id);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.album_tracks (album_id, disc_number, "position", track_id)
        VALUES (NEW.album_id, NEW.disc_number, NEW."position", NEW.track_id)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.album_tracks still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.album_tracks FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_album_tracks();
ALTER TABLE public.album_tracks ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.album_tracks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.album_tracks ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.albums FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_albums', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_albums()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
    k0 boolean;
    k1 boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'albums' AND p.pk = jsonb_build_object('id', OLD.id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.id IS DISTINCT FROM OLD.id THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'albums' AND p.pk = jsonb_build_object('id', OLD.id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'albums' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                    INSERT INTO public.albums (aggregates_updated_at, confidence, cover_url, created_at, genius_album_id, id, is_star_artist, mb_release_id, normalized_title, popularity_score, primary_artist_id, release_date, release_year, source, spotify_album_id, title, total_duration_ms, track_count, type, updated_at) VALUES (NEW.aggregates_updated_at, NEW.confidence, NEW.cover_url, NEW.created_at, NEW.genius_album_id, NEW.id, NEW.is_star_artist, NEW.mb_release_id, NEW.normalized_title, NEW.popularity_score, NEW.primary_artist_id, NEW.release_date, NEW.release_year, NEW.source, NEW.spotify_album_id, NEW.title, NEW.total_duration_ms, NEW.track_count, NEW.type, NEW.updated_at);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'albums' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                DELETE FROM public.albums t WHERE t.id = OLD.id;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'albums' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        k0 := (NEW.mb_release_id, ((NEW.mb_release_id IS NOT NULL))) IS DISTINCT FROM (OLD.mb_release_id, ((OLD.mb_release_id IS NOT NULL)));
        k1 := (NEW.spotify_album_id, ((NEW.spotify_album_id IS NOT NULL))) IS DISTINCT FROM (OLD.spotify_album_id, ((OLD.spotify_album_id IS NOT NULL)));
        IF k0 AND ((NEW.mb_release_id IS NOT NULL)) THEN
            FOR r IN SELECT t.id FROM public.albums t WHERE t.mb_release_id = NEW.mb_release_id AND ((t.mb_release_id IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_albums(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        IF k1 AND ((NEW.spotify_album_id IS NOT NULL)) THEN
            FOR r IN SELECT t.id FROM public.albums t WHERE t.spotify_album_id = NEW.spotify_album_id AND ((t.spotify_album_id IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_albums(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.albums t WHERE t.id = NEW.id FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'albums' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
            RETURN NULL;
        END IF;
        IF ((NEW.mb_release_id IS NOT NULL)) THEN
            FOR r IN SELECT t.id FROM public.albums t WHERE t.mb_release_id = NEW.mb_release_id AND ((t.mb_release_id IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_albums(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        IF ((NEW.spotify_album_id IS NOT NULL)) THEN
            FOR r IN SELECT t.id FROM public.albums t WHERE t.spotify_album_id = NEW.spotify_album_id AND ((t.spotify_album_id IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_albums(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        IF held THEN
            UPDATE public.albums t SET aggregates_updated_at = NEW.aggregates_updated_at, confidence = NEW.confidence, cover_url = NEW.cover_url, created_at = NEW.created_at, genius_album_id = NEW.genius_album_id, is_star_artist = NEW.is_star_artist, mb_release_id = NEW.mb_release_id, normalized_title = NEW.normalized_title, popularity_score = NEW.popularity_score, primary_artist_id = NEW.primary_artist_id, release_date = NEW.release_date, release_year = NEW.release_year, source = NEW.source, spotify_album_id = NEW.spotify_album_id, title = NEW.title, total_duration_ms = NEW.total_duration_ms, track_count = NEW.track_count, type = NEW.type, updated_at = NEW.updated_at WHERE t.id = NEW.id;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.albums (aggregates_updated_at, confidence, cover_url, created_at, genius_album_id, id, is_star_artist, mb_release_id, normalized_title, popularity_score, primary_artist_id, release_date, release_year, source, spotify_album_id, title, total_duration_ms, track_count, type, updated_at)
        VALUES (NEW.aggregates_updated_at, NEW.confidence, NEW.cover_url, NEW.created_at, NEW.genius_album_id, NEW.id, NEW.is_star_artist, NEW.mb_release_id, NEW.normalized_title, NEW.popularity_score, NEW.primary_artist_id, NEW.release_date, NEW.release_year, NEW.source, NEW.spotify_album_id, NEW.title, NEW.total_duration_ms, NEW.track_count, NEW.type, NEW.updated_at)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.albums still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.albums FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_albums();
ALTER TABLE public.albums ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.albums FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.albums ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.artist_attribution_revalidation FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_artist_attribution_revalidation', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_artist_attribution_revalidation()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_attribution_revalidation' AND p.pk = jsonb_build_object('singleton', OLD.singleton)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.singleton IS DISTINCT FROM OLD.singleton THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_attribution_revalidation' AND p.pk = jsonb_build_object('singleton', OLD.singleton)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_attribution_revalidation' AND p.pk = jsonb_build_object('singleton', NEW.singleton)) THEN
                    INSERT INTO public.artist_attribution_revalidation (credits_removed, identity_completed_at, identity_cursor_account, identity_cursor_artist, singleton, tracks_reset, updated_at, walker_completed_at) VALUES (NEW.credits_removed, NEW.identity_completed_at, NEW.identity_cursor_account, NEW.identity_cursor_artist, NEW.singleton, NEW.tracks_reset, NEW.updated_at, NEW.walker_completed_at);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_attribution_revalidation' AND p.pk = jsonb_build_object('singleton', NEW.singleton)) THEN
                DELETE FROM public.artist_attribution_revalidation t WHERE t.singleton = OLD.singleton;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_attribution_revalidation' AND p.pk = jsonb_build_object('singleton', NEW.singleton)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.artist_attribution_revalidation t WHERE t.singleton = NEW.singleton FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_attribution_revalidation' AND p.pk = jsonb_build_object('singleton', NEW.singleton)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.artist_attribution_revalidation t SET credits_removed = NEW.credits_removed, identity_completed_at = NEW.identity_completed_at, identity_cursor_account = NEW.identity_cursor_account, identity_cursor_artist = NEW.identity_cursor_artist, tracks_reset = NEW.tracks_reset, updated_at = NEW.updated_at, walker_completed_at = NEW.walker_completed_at WHERE t.singleton = NEW.singleton;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.artist_attribution_revalidation (credits_removed, identity_completed_at, identity_cursor_account, identity_cursor_artist, singleton, tracks_reset, updated_at, walker_completed_at)
        VALUES (NEW.credits_removed, NEW.identity_completed_at, NEW.identity_cursor_account, NEW.identity_cursor_artist, NEW.singleton, NEW.tracks_reset, NEW.updated_at, NEW.walker_completed_at)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.artist_attribution_revalidation still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.artist_attribution_revalidation FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_artist_attribution_revalidation();
ALTER TABLE public.artist_attribution_revalidation ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.artist_attribution_revalidation FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.artist_attribution_revalidation ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.artist_colike FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_artist_colike', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_artist_colike()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_colike' AND p.pk = jsonb_build_object('a_id', OLD.a_id, 'b_id', OLD.b_id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.a_id, NEW.b_id) IS DISTINCT FROM (OLD.a_id, OLD.b_id) THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_colike' AND p.pk = jsonb_build_object('a_id', OLD.a_id, 'b_id', OLD.b_id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_colike' AND p.pk = jsonb_build_object('a_id', NEW.a_id, 'b_id', NEW.b_id)) THEN
                    INSERT INTO public.artist_colike (a_id, b_id, co, updated_at, w) VALUES (NEW.a_id, NEW.b_id, NEW.co, NEW.updated_at, NEW.w);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_colike' AND p.pk = jsonb_build_object('a_id', NEW.a_id, 'b_id', NEW.b_id)) THEN
                DELETE FROM public.artist_colike t WHERE (t.a_id, t.b_id) = (OLD.a_id, OLD.b_id);
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_colike' AND p.pk = jsonb_build_object('a_id', NEW.a_id, 'b_id', NEW.b_id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.artist_colike t WHERE (t.a_id, t.b_id) = (NEW.a_id, NEW.b_id) FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_colike' AND p.pk = jsonb_build_object('a_id', NEW.a_id, 'b_id', NEW.b_id)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.artist_colike t SET co = NEW.co, updated_at = NEW.updated_at, w = NEW.w WHERE (t.a_id, t.b_id) = (NEW.a_id, NEW.b_id);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.artist_colike (a_id, b_id, co, updated_at, w)
        VALUES (NEW.a_id, NEW.b_id, NEW.co, NEW.updated_at, NEW.w)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.artist_colike still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.artist_colike FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_artist_colike();
ALTER TABLE public.artist_colike ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.artist_colike FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.artist_colike ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.artist_coplay FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_artist_coplay', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_artist_coplay()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_coplay' AND p.pk = jsonb_build_object('a_id', OLD.a_id, 'b_id', OLD.b_id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.a_id, NEW.b_id) IS DISTINCT FROM (OLD.a_id, OLD.b_id) THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_coplay' AND p.pk = jsonb_build_object('a_id', OLD.a_id, 'b_id', OLD.b_id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_coplay' AND p.pk = jsonb_build_object('a_id', NEW.a_id, 'b_id', NEW.b_id)) THEN
                    INSERT INTO public.artist_coplay (a_id, b_id, last_seen, weight) VALUES (NEW.a_id, NEW.b_id, NEW.last_seen, NEW.weight);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_coplay' AND p.pk = jsonb_build_object('a_id', NEW.a_id, 'b_id', NEW.b_id)) THEN
                DELETE FROM public.artist_coplay t WHERE (t.a_id, t.b_id) = (OLD.a_id, OLD.b_id);
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_coplay' AND p.pk = jsonb_build_object('a_id', NEW.a_id, 'b_id', NEW.b_id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.artist_coplay t WHERE (t.a_id, t.b_id) = (NEW.a_id, NEW.b_id) FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_coplay' AND p.pk = jsonb_build_object('a_id', NEW.a_id, 'b_id', NEW.b_id)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.artist_coplay t SET last_seen = NEW.last_seen, weight = NEW.weight WHERE (t.a_id, t.b_id) = (NEW.a_id, NEW.b_id);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.artist_coplay (a_id, b_id, last_seen, weight)
        VALUES (NEW.a_id, NEW.b_id, NEW.last_seen, NEW.weight)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.artist_coplay still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.artist_coplay FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_artist_coplay();
ALTER TABLE public.artist_coplay ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.artist_coplay FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.artist_coplay ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.artist_sc_accounts FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_artist_sc_accounts', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_artist_sc_accounts()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
    k0 boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_sc_accounts' AND p.pk = jsonb_build_object('artist_id', OLD.artist_id, 'sc_user_id', OLD.sc_user_id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.artist_id, NEW.sc_user_id) IS DISTINCT FROM (OLD.artist_id, OLD.sc_user_id) THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_sc_accounts' AND p.pk = jsonb_build_object('artist_id', OLD.artist_id, 'sc_user_id', OLD.sc_user_id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_sc_accounts' AND p.pk = jsonb_build_object('artist_id', NEW.artist_id, 'sc_user_id', NEW.sc_user_id)) THEN
                    INSERT INTO public.artist_sc_accounts (artist_id, created_at, notes, role, sc_user_id, source, updated_at, verified) VALUES (NEW.artist_id, NEW.created_at, NEW.notes, NEW.role, NEW.sc_user_id, NEW.source, NEW.updated_at, NEW.verified);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_sc_accounts' AND p.pk = jsonb_build_object('artist_id', NEW.artist_id, 'sc_user_id', NEW.sc_user_id)) THEN
                DELETE FROM public.artist_sc_accounts t WHERE (t.artist_id, t.sc_user_id) = (OLD.artist_id, OLD.sc_user_id);
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_sc_accounts' AND p.pk = jsonb_build_object('artist_id', NEW.artist_id, 'sc_user_id', NEW.sc_user_id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        k0 := (NEW.sc_user_id, (NEW.verified)) IS DISTINCT FROM (OLD.sc_user_id, (OLD.verified));
        IF k0 AND (NEW.verified) THEN
            FOR r IN SELECT t.artist_id, t.sc_user_id FROM public.artist_sc_accounts t WHERE t.sc_user_id = NEW.sc_user_id AND (t.verified) AND (t.artist_id, t.sc_user_id) IS DISTINCT FROM (NEW.artist_id, NEW.sc_user_id) LOOP
                PERFORM public.scd_purge_artist_sc_accounts(jsonb_build_object('artist_id', r.artist_id, 'sc_user_id', r.sc_user_id));
            END LOOP;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.artist_sc_accounts t WHERE (t.artist_id, t.sc_user_id) = (NEW.artist_id, NEW.sc_user_id) FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_sc_accounts' AND p.pk = jsonb_build_object('artist_id', NEW.artist_id, 'sc_user_id', NEW.sc_user_id)) THEN
            RETURN NULL;
        END IF;
        IF (NEW.verified) THEN
            FOR r IN SELECT t.artist_id, t.sc_user_id FROM public.artist_sc_accounts t WHERE t.sc_user_id = NEW.sc_user_id AND (t.verified) AND (t.artist_id, t.sc_user_id) IS DISTINCT FROM (NEW.artist_id, NEW.sc_user_id) LOOP
                PERFORM public.scd_purge_artist_sc_accounts(jsonb_build_object('artist_id', r.artist_id, 'sc_user_id', r.sc_user_id));
            END LOOP;
        END IF;
        IF held THEN
            UPDATE public.artist_sc_accounts t SET created_at = NEW.created_at, notes = NEW.notes, role = NEW.role, source = NEW.source, updated_at = NEW.updated_at, verified = NEW.verified WHERE (t.artist_id, t.sc_user_id) = (NEW.artist_id, NEW.sc_user_id);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.artist_sc_accounts (artist_id, created_at, notes, role, sc_user_id, source, updated_at, verified)
        VALUES (NEW.artist_id, NEW.created_at, NEW.notes, NEW.role, NEW.sc_user_id, NEW.source, NEW.updated_at, NEW.verified)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.artist_sc_accounts still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.artist_sc_accounts FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_artist_sc_accounts();
ALTER TABLE public.artist_sc_accounts ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.artist_sc_accounts FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.artist_sc_accounts ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.artist_socials FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_artist_socials', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_artist_socials()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_socials' AND p.pk = jsonb_build_object('artist_id', OLD.artist_id, 'url', OLD.url)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.artist_id, NEW.url) IS DISTINCT FROM (OLD.artist_id, OLD.url) THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_socials' AND p.pk = jsonb_build_object('artist_id', OLD.artist_id, 'url', OLD.url)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_socials' AND p.pk = jsonb_build_object('artist_id', NEW.artist_id, 'url', NEW.url)) THEN
                    INSERT INTO public.artist_socials (artist_id, created_at, kind, source, updated_at, url, verified) VALUES (NEW.artist_id, NEW.created_at, NEW.kind, NEW.source, NEW.updated_at, NEW.url, NEW.verified);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_socials' AND p.pk = jsonb_build_object('artist_id', NEW.artist_id, 'url', NEW.url)) THEN
                DELETE FROM public.artist_socials t WHERE (t.artist_id, t.url) = (OLD.artist_id, OLD.url);
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_socials' AND p.pk = jsonb_build_object('artist_id', NEW.artist_id, 'url', NEW.url)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.artist_socials t WHERE (t.artist_id, t.url) = (NEW.artist_id, NEW.url) FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_socials' AND p.pk = jsonb_build_object('artist_id', NEW.artist_id, 'url', NEW.url)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.artist_socials t SET created_at = NEW.created_at, kind = NEW.kind, source = NEW.source, updated_at = NEW.updated_at, verified = NEW.verified WHERE (t.artist_id, t.url) = (NEW.artist_id, NEW.url);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.artist_socials (artist_id, created_at, kind, source, updated_at, url, verified)
        VALUES (NEW.artist_id, NEW.created_at, NEW.kind, NEW.source, NEW.updated_at, NEW.url, NEW.verified)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.artist_socials still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.artist_socials FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_artist_socials();
ALTER TABLE public.artist_socials ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.artist_socials FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.artist_socials ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_core_col_artists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF NEW.has_sc_account IS DISTINCT FROM OLD.has_sc_account THEN
        NEW.has_sc_account := OLD.has_sc_account;
        PERFORM nextval('public.scd_core_col_artists'::regclass);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_core_col BEFORE UPDATE OF has_sc_account ON public.artists FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_core_col_artists();

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.artists FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_artists', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_artists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
    k0 boolean;
    k1 boolean;
    k2 boolean;
    k3 boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artists' AND p.pk = jsonb_build_object('id', OLD.id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.id IS DISTINCT FROM OLD.id THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artists' AND p.pk = jsonb_build_object('id', OLD.id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artists' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                    INSERT INTO public.artists (account_walk_locked_at, aggregates_updated_at, album_count_denorm, avatar_url, bio, confidence, country, crawl_attempts, crawl_dead, crawl_fail_count, created_at, genius_artist_id, genius_crawl_offset, genius_crawled_at, genius_locked_at, genius_next_run_at, has_sc_account, id, interest_score, is_star, isni, last_account_walk_at, last_crawled_at, mb_artist_id, mb_crawl_offset, mb_crawled_at, mb_locked_at, mb_next_run_at, merged_into, monthly_listeners, name, normalized_name, popularity_score, sc_user_id, source, spotify_artist_id, star_aura_id, star_custom_hex, tags, track_count_featured, track_count_primary, trending_score, updated_at) VALUES (NEW.account_walk_locked_at, NEW.aggregates_updated_at, NEW.album_count_denorm, NEW.avatar_url, NEW.bio, NEW.confidence, NEW.country, NEW.crawl_attempts, NEW.crawl_dead, NEW.crawl_fail_count, NEW.created_at, NEW.genius_artist_id, NEW.genius_crawl_offset, NEW.genius_crawled_at, NEW.genius_locked_at, NEW.genius_next_run_at, NEW.has_sc_account, NEW.id, NEW.interest_score, NEW.is_star, NEW.isni, NEW.last_account_walk_at, NEW.last_crawled_at, NEW.mb_artist_id, NEW.mb_crawl_offset, NEW.mb_crawled_at, NEW.mb_locked_at, NEW.mb_next_run_at, NEW.merged_into, NEW.monthly_listeners, NEW.name, NEW.normalized_name, NEW.popularity_score, NEW.sc_user_id, NEW.source, NEW.spotify_artist_id, NEW.star_aura_id, NEW.star_custom_hex, NEW.tags, NEW.track_count_featured, NEW.track_count_primary, NEW.trending_score, NEW.updated_at);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artists' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                DELETE FROM public.artists t WHERE t.id = OLD.id;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artists' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                IF NEW.has_sc_account IS DISTINCT FROM OLD.has_sc_account THEN
                    NEW.account_walk_locked_at := OLD.account_walk_locked_at;
                    NEW.aggregates_updated_at := OLD.aggregates_updated_at;
                    NEW.album_count_denorm := OLD.album_count_denorm;
                    NEW.avatar_url := OLD.avatar_url;
                    NEW.bio := OLD.bio;
                    NEW.confidence := OLD.confidence;
                    NEW.country := OLD.country;
                    NEW.crawl_attempts := OLD.crawl_attempts;
                    NEW.crawl_dead := OLD.crawl_dead;
                    NEW.crawl_fail_count := OLD.crawl_fail_count;
                    NEW.created_at := OLD.created_at;
                    NEW.genius_artist_id := OLD.genius_artist_id;
                    NEW.genius_crawl_offset := OLD.genius_crawl_offset;
                    NEW.genius_crawled_at := OLD.genius_crawled_at;
                    NEW.genius_locked_at := OLD.genius_locked_at;
                    NEW.genius_next_run_at := OLD.genius_next_run_at;
                    NEW.interest_score := OLD.interest_score;
                    NEW.is_star := OLD.is_star;
                    NEW.isni := OLD.isni;
                    NEW.last_account_walk_at := OLD.last_account_walk_at;
                    NEW.last_crawled_at := OLD.last_crawled_at;
                    NEW.mb_artist_id := OLD.mb_artist_id;
                    NEW.mb_crawl_offset := OLD.mb_crawl_offset;
                    NEW.mb_crawled_at := OLD.mb_crawled_at;
                    NEW.mb_locked_at := OLD.mb_locked_at;
                    NEW.mb_next_run_at := OLD.mb_next_run_at;
                    NEW.merged_into := OLD.merged_into;
                    NEW.monthly_listeners := OLD.monthly_listeners;
                    NEW.name := OLD.name;
                    NEW.normalized_name := OLD.normalized_name;
                    NEW.popularity_score := OLD.popularity_score;
                    NEW.sc_user_id := OLD.sc_user_id;
                    NEW.source := OLD.source;
                    NEW.spotify_artist_id := OLD.spotify_artist_id;
                    NEW.star_aura_id := OLD.star_aura_id;
                    NEW.star_custom_hex := OLD.star_custom_hex;
                    NEW.tags := OLD.tags;
                    NEW.track_count_featured := OLD.track_count_featured;
                    NEW.track_count_primary := OLD.track_count_primary;
                    NEW.trending_score := OLD.trending_score;
                    NEW.updated_at := OLD.updated_at;
                    RETURN NEW;
                END IF;
                RETURN NULL;
            END IF;
        END IF;
        k0 := (NEW.genius_artist_id, ((NEW.genius_artist_id IS NOT NULL))) IS DISTINCT FROM (OLD.genius_artist_id, ((OLD.genius_artist_id IS NOT NULL)));
        k1 := (NEW.mb_artist_id, ((NEW.mb_artist_id IS NOT NULL))) IS DISTINCT FROM (OLD.mb_artist_id, ((OLD.mb_artist_id IS NOT NULL)));
        k2 := (NEW.normalized_name, ((NEW.merged_into IS NULL))) IS DISTINCT FROM (OLD.normalized_name, ((OLD.merged_into IS NULL)));
        k3 := (NEW.spotify_artist_id, ((NEW.spotify_artist_id IS NOT NULL))) IS DISTINCT FROM (OLD.spotify_artist_id, ((OLD.spotify_artist_id IS NOT NULL)));
        IF k0 AND ((NEW.genius_artist_id IS NOT NULL)) THEN
            FOR r IN SELECT t.id FROM public.artists t WHERE t.genius_artist_id = NEW.genius_artist_id AND ((t.genius_artist_id IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_artists(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        IF k1 AND ((NEW.mb_artist_id IS NOT NULL)) THEN
            FOR r IN SELECT t.id FROM public.artists t WHERE t.mb_artist_id = NEW.mb_artist_id AND ((t.mb_artist_id IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_artists(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        IF k2 AND ((NEW.merged_into IS NULL)) THEN
            FOR r IN SELECT t.id FROM public.artists t WHERE t.normalized_name = NEW.normalized_name AND ((t.merged_into IS NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_artists(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        IF k3 AND ((NEW.spotify_artist_id IS NOT NULL)) THEN
            FOR r IN SELECT t.id FROM public.artists t WHERE t.spotify_artist_id = NEW.spotify_artist_id AND ((t.spotify_artist_id IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_artists(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.artists t WHERE t.id = NEW.id FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artists' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
            PERFORM pg_catalog.set_config('scd.merging', 'on', true);
            UPDATE public.artists t SET has_sc_account = NEW.has_sc_account WHERE t.id = NEW.id AND t.has_sc_account IS DISTINCT FROM NEW.has_sc_account;
            PERFORM pg_catalog.set_config('scd.merging', '', true);
            RETURN NULL;
        END IF;
        IF ((NEW.genius_artist_id IS NOT NULL)) THEN
            FOR r IN SELECT t.id FROM public.artists t WHERE t.genius_artist_id = NEW.genius_artist_id AND ((t.genius_artist_id IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_artists(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        IF ((NEW.mb_artist_id IS NOT NULL)) THEN
            FOR r IN SELECT t.id FROM public.artists t WHERE t.mb_artist_id = NEW.mb_artist_id AND ((t.mb_artist_id IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_artists(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        IF ((NEW.merged_into IS NULL)) THEN
            FOR r IN SELECT t.id FROM public.artists t WHERE t.normalized_name = NEW.normalized_name AND ((t.merged_into IS NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_artists(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        IF ((NEW.spotify_artist_id IS NOT NULL)) THEN
            FOR r IN SELECT t.id FROM public.artists t WHERE t.spotify_artist_id = NEW.spotify_artist_id AND ((t.spotify_artist_id IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_artists(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        IF held THEN
            UPDATE public.artists t SET account_walk_locked_at = NEW.account_walk_locked_at, aggregates_updated_at = NEW.aggregates_updated_at, album_count_denorm = NEW.album_count_denorm, avatar_url = NEW.avatar_url, bio = NEW.bio, confidence = NEW.confidence, country = NEW.country, crawl_attempts = NEW.crawl_attempts, crawl_dead = NEW.crawl_dead, crawl_fail_count = NEW.crawl_fail_count, created_at = NEW.created_at, genius_artist_id = NEW.genius_artist_id, genius_crawl_offset = NEW.genius_crawl_offset, genius_crawled_at = NEW.genius_crawled_at, genius_locked_at = NEW.genius_locked_at, genius_next_run_at = NEW.genius_next_run_at, has_sc_account = NEW.has_sc_account, interest_score = NEW.interest_score, is_star = NEW.is_star, isni = NEW.isni, last_account_walk_at = NEW.last_account_walk_at, last_crawled_at = NEW.last_crawled_at, mb_artist_id = NEW.mb_artist_id, mb_crawl_offset = NEW.mb_crawl_offset, mb_crawled_at = NEW.mb_crawled_at, mb_locked_at = NEW.mb_locked_at, mb_next_run_at = NEW.mb_next_run_at, merged_into = NEW.merged_into, monthly_listeners = NEW.monthly_listeners, name = NEW.name, normalized_name = NEW.normalized_name, popularity_score = NEW.popularity_score, sc_user_id = NEW.sc_user_id, source = NEW.source, spotify_artist_id = NEW.spotify_artist_id, star_aura_id = NEW.star_aura_id, star_custom_hex = NEW.star_custom_hex, tags = NEW.tags, track_count_featured = NEW.track_count_featured, track_count_primary = NEW.track_count_primary, trending_score = NEW.trending_score, updated_at = NEW.updated_at WHERE t.id = NEW.id;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.artists (account_walk_locked_at, aggregates_updated_at, album_count_denorm, avatar_url, bio, confidence, country, crawl_attempts, crawl_dead, crawl_fail_count, created_at, genius_artist_id, genius_crawl_offset, genius_crawled_at, genius_locked_at, genius_next_run_at, has_sc_account, id, interest_score, is_star, isni, last_account_walk_at, last_crawled_at, mb_artist_id, mb_crawl_offset, mb_crawled_at, mb_locked_at, mb_next_run_at, merged_into, monthly_listeners, name, normalized_name, popularity_score, sc_user_id, source, spotify_artist_id, star_aura_id, star_custom_hex, tags, track_count_featured, track_count_primary, trending_score, updated_at)
        VALUES (NEW.account_walk_locked_at, NEW.aggregates_updated_at, NEW.album_count_denorm, NEW.avatar_url, NEW.bio, NEW.confidence, NEW.country, NEW.crawl_attempts, NEW.crawl_dead, NEW.crawl_fail_count, NEW.created_at, NEW.genius_artist_id, NEW.genius_crawl_offset, NEW.genius_crawled_at, NEW.genius_locked_at, NEW.genius_next_run_at, NEW.has_sc_account, NEW.id, NEW.interest_score, NEW.is_star, NEW.isni, NEW.last_account_walk_at, NEW.last_crawled_at, NEW.mb_artist_id, NEW.mb_crawl_offset, NEW.mb_crawled_at, NEW.mb_locked_at, NEW.mb_next_run_at, NEW.merged_into, NEW.monthly_listeners, NEW.name, NEW.normalized_name, NEW.popularity_score, NEW.sc_user_id, NEW.source, NEW.spotify_artist_id, NEW.star_aura_id, NEW.star_custom_hex, NEW.tags, NEW.track_count_featured, NEW.track_count_primary, NEW.trending_score, NEW.updated_at)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.artists still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.artists FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_artists();
ALTER TABLE public.artists ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.artists FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.artists ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.audio_index_wire_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_audio_index_wire_state', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.background_job_enqueues FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_background_job_enqueues', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.background_job_failures FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_background_job_failures', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_background_job_failures()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'background_job_failures' AND p.pk = jsonb_build_object('id', OLD.id, 'generation', OLD.generation)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.id, NEW.generation) IS DISTINCT FROM (OLD.id, OLD.generation) THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'background_job_failures' AND p.pk = jsonb_build_object('id', OLD.id, 'generation', OLD.generation)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'background_job_failures' AND p.pk = jsonb_build_object('id', NEW.id, 'generation', NEW.generation)) THEN
                    INSERT INTO public.background_job_failures (attempts, created_at, dedup_key, failed_at, generation, id, kind, lane, last_error, max_attempts, payload, priority) VALUES (NEW.attempts, NEW.created_at, NEW.dedup_key, NEW.failed_at, NEW.generation, NEW.id, NEW.kind, NEW.lane, NEW.last_error, NEW.max_attempts, NEW.payload, NEW.priority);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'background_job_failures' AND p.pk = jsonb_build_object('id', NEW.id, 'generation', NEW.generation)) THEN
                DELETE FROM public.background_job_failures t WHERE (t.id, t.generation) = (OLD.id, OLD.generation);
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'background_job_failures' AND p.pk = jsonb_build_object('id', NEW.id, 'generation', NEW.generation)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.background_job_failures t WHERE (t.id, t.generation) = (NEW.id, NEW.generation) FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'background_job_failures' AND p.pk = jsonb_build_object('id', NEW.id, 'generation', NEW.generation)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.background_job_failures t SET attempts = NEW.attempts, created_at = NEW.created_at, dedup_key = NEW.dedup_key, failed_at = NEW.failed_at, kind = NEW.kind, lane = NEW.lane, last_error = NEW.last_error, max_attempts = NEW.max_attempts, payload = NEW.payload, priority = NEW.priority WHERE (t.id, t.generation) = (NEW.id, NEW.generation);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.background_job_failures (attempts, created_at, dedup_key, failed_at, generation, id, kind, lane, last_error, max_attempts, payload, priority)
        VALUES (NEW.attempts, NEW.created_at, NEW.dedup_key, NEW.failed_at, NEW.generation, NEW.id, NEW.kind, NEW.lane, NEW.last_error, NEW.max_attempts, NEW.payload, NEW.priority)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.background_job_failures still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.background_job_failures FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_background_job_failures();
ALTER TABLE public.background_job_failures ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.background_job_failures FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.background_job_failures ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.background_jobs FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_background_jobs', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.background_schedules FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_background_schedules', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.catalog_audience FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_catalog_audience', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_catalog_audience()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'catalog_audience' AND p.pk = jsonb_build_object('subject_urn', OLD.subject_urn, 'relation', OLD.relation, 'user_urn', OLD.user_urn)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.subject_urn, NEW.relation, NEW.user_urn) IS DISTINCT FROM (OLD.subject_urn, OLD.relation, OLD.user_urn) THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'catalog_audience' AND p.pk = jsonb_build_object('subject_urn', OLD.subject_urn, 'relation', OLD.relation, 'user_urn', OLD.user_urn)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'catalog_audience' AND p.pk = jsonb_build_object('subject_urn', NEW.subject_urn, 'relation', NEW.relation, 'user_urn', NEW.user_urn)) THEN
                    INSERT INTO public.catalog_audience (created_at, relation, subject_urn, synced_at, user_urn) VALUES (NEW.created_at, NEW.relation, NEW.subject_urn, NEW.synced_at, NEW.user_urn);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'catalog_audience' AND p.pk = jsonb_build_object('subject_urn', NEW.subject_urn, 'relation', NEW.relation, 'user_urn', NEW.user_urn)) THEN
                DELETE FROM public.catalog_audience t WHERE (t.subject_urn, t.relation, t.user_urn) = (OLD.subject_urn, OLD.relation, OLD.user_urn);
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'catalog_audience' AND p.pk = jsonb_build_object('subject_urn', NEW.subject_urn, 'relation', NEW.relation, 'user_urn', NEW.user_urn)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.catalog_audience t WHERE (t.subject_urn, t.relation, t.user_urn) = (NEW.subject_urn, NEW.relation, NEW.user_urn) FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'catalog_audience' AND p.pk = jsonb_build_object('subject_urn', NEW.subject_urn, 'relation', NEW.relation, 'user_urn', NEW.user_urn)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.catalog_audience t SET created_at = NEW.created_at, synced_at = NEW.synced_at WHERE (t.subject_urn, t.relation, t.user_urn) = (NEW.subject_urn, NEW.relation, NEW.user_urn);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.catalog_audience (created_at, relation, subject_urn, synced_at, user_urn)
        VALUES (NEW.created_at, NEW.relation, NEW.subject_urn, NEW.synced_at, NEW.user_urn)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.catalog_audience still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.catalog_audience FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_catalog_audience();
ALTER TABLE public.catalog_audience ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.catalog_audience FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.catalog_audience ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.catalog_collection_cursors FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_catalog_collection_cursors', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.catalog_collection_seen FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_catalog_collection_seen', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.catalog_collection_sync FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_catalog_collection_sync', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_catalog_collection_sync()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'catalog_collection_sync' AND p.pk = jsonb_build_object('subject_id', OLD.subject_id, 'collection', OLD.collection, 'scope', OLD.scope)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.subject_id, NEW.collection, NEW.scope) IS DISTINCT FROM (OLD.subject_id, OLD.collection, OLD.scope) THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'catalog_collection_sync' AND p.pk = jsonb_build_object('subject_id', OLD.subject_id, 'collection', OLD.collection, 'scope', OLD.scope)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'catalog_collection_sync' AND p.pk = jsonb_build_object('subject_id', NEW.subject_id, 'collection', NEW.collection, 'scope', NEW.scope)) THEN
                    INSERT INTO public.catalog_collection_sync (collection, complete, generation, item_count, job_id, next_cursor, page_count, scope, snapshot_id, started_at, subject_id, synced_at, updated_at, verified_at) VALUES (NEW.collection, NEW.complete, NEW.generation, NEW.item_count, NEW.job_id, NEW.next_cursor, NEW.page_count, NEW.scope, NEW.snapshot_id, NEW.started_at, NEW.subject_id, NEW.synced_at, NEW.updated_at, NEW.verified_at);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'catalog_collection_sync' AND p.pk = jsonb_build_object('subject_id', NEW.subject_id, 'collection', NEW.collection, 'scope', NEW.scope)) THEN
                DELETE FROM public.catalog_collection_sync t WHERE (t.subject_id, t.collection, t.scope) = (OLD.subject_id, OLD.collection, OLD.scope);
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'catalog_collection_sync' AND p.pk = jsonb_build_object('subject_id', NEW.subject_id, 'collection', NEW.collection, 'scope', NEW.scope)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.catalog_collection_sync t WHERE (t.subject_id, t.collection, t.scope) = (NEW.subject_id, NEW.collection, NEW.scope) FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'catalog_collection_sync' AND p.pk = jsonb_build_object('subject_id', NEW.subject_id, 'collection', NEW.collection, 'scope', NEW.scope)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.catalog_collection_sync t SET complete = NEW.complete, generation = NEW.generation, item_count = NEW.item_count, job_id = NEW.job_id, next_cursor = NEW.next_cursor, page_count = NEW.page_count, snapshot_id = NEW.snapshot_id, started_at = NEW.started_at, synced_at = NEW.synced_at, updated_at = NEW.updated_at, verified_at = NEW.verified_at WHERE (t.subject_id, t.collection, t.scope) = (NEW.subject_id, NEW.collection, NEW.scope);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.catalog_collection_sync (collection, complete, generation, item_count, job_id, next_cursor, page_count, scope, snapshot_id, started_at, subject_id, synced_at, updated_at, verified_at)
        VALUES (NEW.collection, NEW.complete, NEW.generation, NEW.item_count, NEW.job_id, NEW.next_cursor, NEW.page_count, NEW.scope, NEW.snapshot_id, NEW.started_at, NEW.subject_id, NEW.synced_at, NEW.updated_at, NEW.verified_at)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.catalog_collection_sync still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.catalog_collection_sync FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_catalog_collection_sync();
ALTER TABLE public.catalog_collection_sync ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.catalog_collection_sync FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.catalog_collection_sync ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.catalog_merge_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_catalog_merge_state', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.catalog_work_links FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_catalog_work_links', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_merge_cdn_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.cdn_path := OLD.cdn_path;
            NEW.created_at := OLD.created_at;
            NEW.quality := OLD.quality;
            NEW.status := OLD.status;
            NEW.stored_quality := OLD.stored_quality;
            NEW.track_urn := OLD.track_urn;
            NEW.updated_at := OLD.updated_at;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.cdn_tracks t WHERE t.id = NEW.id;
        IF FOUND THEN
            IF NEW.row_v > o.row_v OR (((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) AND NEW.created_at < o.created_at) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.cdn_tracks t SET
                    cdn_path = CASE WHEN NEW.row_v > o.row_v THEN NEW.cdn_path ELSE o.cdn_path END,
                    created_at = CASE WHEN ((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.row_v > o.row_v THEN NEW.created_at ELSE o.created_at END,
                    quality = CASE WHEN NEW.row_v > o.row_v THEN NEW.quality ELSE o.quality END,
                    status = CASE WHEN NEW.row_v > o.row_v THEN NEW.status ELSE o.status END,
                    stored_quality = CASE WHEN NEW.row_v > o.row_v THEN NEW.stored_quality ELSE o.stored_quality END,
                    track_urn = CASE WHEN NEW.row_v > o.row_v THEN NEW.track_urn ELSE o.track_urn END,
                    updated_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.updated_at ELSE o.updated_at END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE t.id = o.id;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        SELECT t.* INTO o FROM public.cdn_tracks t WHERE (t.track_urn, t.quality) = (NEW.track_urn, NEW.quality);
        IF FOUND THEN
            PERFORM nextval('public.scd_conflict_cdn_tracks'::regclass);
            IF NEW.row_v > o.row_v OR (((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) AND NEW.created_at < o.created_at) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.cdn_tracks t SET
                    cdn_path = CASE WHEN NEW.row_v > o.row_v THEN NEW.cdn_path ELSE o.cdn_path END,
                    created_at = CASE WHEN ((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.row_v > o.row_v THEN NEW.created_at ELSE o.created_at END,
                    quality = CASE WHEN NEW.row_v > o.row_v THEN NEW.quality ELSE o.quality END,
                    status = CASE WHEN NEW.row_v > o.row_v THEN NEW.status ELSE o.status END,
                    stored_quality = CASE WHEN NEW.row_v > o.row_v THEN NEW.stored_quality ELSE o.stored_quality END,
                    track_urn = CASE WHEN NEW.row_v > o.row_v THEN NEW.track_urn ELSE o.track_urn END,
                    updated_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.updated_at ELSE o.updated_at END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE t.id = o.id;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.cdn_tracks (cdn_path, created_at, id, last_accessed_at, quality, row_v, status, stored_quality, track_urn, updated_at)
        VALUES (NEW.cdn_path, NEW.created_at, NEW.id, NEW.last_accessed_at, NEW.quality, NEW.row_v, NEW.status, NEW.stored_quality, NEW.track_urn, NEW.updated_at)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.cdn_tracks still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.cdn_tracks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_cdn_tracks();
ALTER TABLE public.cdn_tracks ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_ver_cdn_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.cdn_path, NEW.created_at, NEW.quality, NEW.status, NEW.stored_quality, NEW.track_urn, NEW.updated_at) IS DISTINCT FROM (OLD.cdn_path, OLD.created_at, OLD.quality, OLD.status, OLD.stored_quality, OLD.track_urn, OLD.updated_at) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.cdn_tracks FOR EACH ROW EXECUTE FUNCTION public.scd_ver_cdn_tracks();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.cdn_tracks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.cdn_tracks ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.cmd_dead FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_cmd_dead', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_skip_diag_bundles()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    PERFORM 1 FROM public.diag_bundles t WHERE t.code = NEW.code;
    IF FOUND THEN
        RETURN NULL;
    END IF;
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    INSERT INTO public.diag_bundles (app_version, at, body, code, os)
    VALUES (NEW.app_version, NEW.at, NEW.body, NEW.code, NEW.os)
    ON CONFLICT DO NOTHING;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_skip BEFORE INSERT ON public.diag_bundles FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_skip_diag_bundles();
ALTER TABLE public.diag_bundles ENABLE REPLICA TRIGGER zz_skip;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.diag_bundles FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.diag_bundles ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_skip_diag_events()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    PERFORM 1 FROM public.diag_events t WHERE t.id = NEW.id;
    IF FOUND THEN
        RETURN NULL;
    END IF;
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    INSERT INTO public.diag_events (app_version, at, body, fingerprint, id, install, kind, os, title, trace)
    VALUES (NEW.app_version, NEW.at, NEW.body, NEW.fingerprint, NEW.id, NEW.install, NEW.kind, NEW.os, NEW.title, NEW.trace)
    ON CONFLICT DO NOTHING;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_skip BEFORE INSERT ON public.diag_events FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_skip_diag_events();
ALTER TABLE public.diag_events ENABLE REPLICA TRIGGER zz_skip;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.diag_events FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.diag_events ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_merge_diag_issue_days()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.events := OLD.events;
            NEW.first_at := OLD.first_at;
            NEW.kind := OLD.kind;
            NEW.last_at := OLD.last_at;
            NEW.title := OLD.title;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.diag_issue_days t WHERE (t.fingerprint, t.day, t.host, t.app_version) = (NEW.fingerprint, NEW.day, NEW.host, NEW.app_version);
        IF FOUND THEN
            IF NEW.row_v > o.row_v THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.diag_issue_days t SET
                    events = CASE WHEN NEW.row_v > o.row_v THEN NEW.events ELSE o.events END,
                    first_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.first_at ELSE o.first_at END,
                    kind = CASE WHEN NEW.row_v > o.row_v THEN NEW.kind ELSE o.kind END,
                    last_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.last_at ELSE o.last_at END,
                    title = CASE WHEN NEW.row_v > o.row_v THEN NEW.title ELSE o.title END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE (t.fingerprint, t.day, t.host, t.app_version) = (o.fingerprint, o.day, o.host, o.app_version);
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.diag_issue_days (app_version, day, events, fingerprint, first_at, host, kind, last_at, row_v, title)
        VALUES (NEW.app_version, NEW.day, NEW.events, NEW.fingerprint, NEW.first_at, NEW.host, NEW.kind, NEW.last_at, NEW.row_v, NEW.title)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.diag_issue_days still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.diag_issue_days FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_diag_issue_days();
ALTER TABLE public.diag_issue_days ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_ver_diag_issue_days()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.events, NEW.first_at, NEW.kind, NEW.last_at, NEW.title) IS DISTINCT FROM (OLD.events, OLD.first_at, OLD.kind, OLD.last_at, OLD.title) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.diag_issue_days FOR EACH ROW EXECUTE FUNCTION public.scd_ver_diag_issue_days();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.diag_issue_days FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.diag_issue_days ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_skip_diag_issue_origins()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    PERFORM 1 FROM public.diag_issue_origins t WHERE (t.fingerprint, t.day, t.host, t.origin) = (NEW.fingerprint, NEW.day, NEW.host, NEW.origin);
    IF FOUND THEN
        RETURN NULL;
    END IF;
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    INSERT INTO public.diag_issue_origins (day, fingerprint, host, origin)
    VALUES (NEW.day, NEW.fingerprint, NEW.host, NEW.origin)
    ON CONFLICT DO NOTHING;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_skip BEFORE INSERT ON public.diag_issue_origins FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_skip_diag_issue_origins();
ALTER TABLE public.diag_issue_origins ENABLE REPLICA TRIGGER zz_skip;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.diag_issue_origins FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.diag_issue_origins ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_merge_discover_promoted()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.active := OLD.active;
            NEW.created_at := OLD.created_at;
            NEW.entity_id := OLD.entity_id;
            NEW.entity_type := OLD.entity_type;
            NEW.note := OLD.note;
            NEW."position" := OLD."position";
            NEW.updated_at := OLD.updated_at;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.discover_promoted t WHERE t.id = NEW.id;
        IF FOUND THEN
            IF NEW.row_v > o.row_v OR (((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) AND NEW.created_at < o.created_at) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.discover_promoted t SET
                    active = CASE WHEN NEW.row_v > o.row_v THEN NEW.active ELSE o.active END,
                    created_at = CASE WHEN ((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.row_v > o.row_v THEN NEW.created_at ELSE o.created_at END,
                    entity_id = CASE WHEN NEW.row_v > o.row_v THEN NEW.entity_id ELSE o.entity_id END,
                    entity_type = CASE WHEN NEW.row_v > o.row_v THEN NEW.entity_type ELSE o.entity_type END,
                    note = CASE WHEN NEW.row_v > o.row_v THEN NEW.note ELSE o.note END,
                    "position" = CASE WHEN NEW.row_v > o.row_v THEN NEW."position" ELSE o."position" END,
                    updated_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.updated_at ELSE o.updated_at END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE t.id = o.id;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        SELECT t.* INTO o FROM public.discover_promoted t WHERE (t.entity_type, t.entity_id) = (NEW.entity_type, NEW.entity_id);
        IF FOUND THEN
            PERFORM nextval('public.scd_conflict_discover_promoted'::regclass);
            IF NEW.row_v > o.row_v OR (((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) AND NEW.created_at < o.created_at) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.discover_promoted t SET
                    active = CASE WHEN NEW.row_v > o.row_v THEN NEW.active ELSE o.active END,
                    created_at = CASE WHEN ((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.row_v > o.row_v THEN NEW.created_at ELSE o.created_at END,
                    entity_id = CASE WHEN NEW.row_v > o.row_v THEN NEW.entity_id ELSE o.entity_id END,
                    entity_type = CASE WHEN NEW.row_v > o.row_v THEN NEW.entity_type ELSE o.entity_type END,
                    note = CASE WHEN NEW.row_v > o.row_v THEN NEW.note ELSE o.note END,
                    "position" = CASE WHEN NEW.row_v > o.row_v THEN NEW."position" ELSE o."position" END,
                    updated_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.updated_at ELSE o.updated_at END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE t.id = o.id;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'discover_promoted' AND x.pk = jsonb_build_object('id', NEW.id));
        IF tv IS NOT NULL AND NEW.row_v <= tv THEN
            INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'discover_promoted', 'pk', jsonb_build_object('id', NEW.id), 'v', tv)) ON CONFLICT DO NOTHING;
            PERFORM nextval('public.scd_resend_discover_promoted'::regclass);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.discover_promoted (active, created_at, entity_id, entity_type, id, note, "position", row_v, updated_at)
        VALUES (NEW.active, NEW.created_at, NEW.entity_id, NEW.entity_type, NEW.id, NEW.note, NEW."position", NEW.row_v, NEW.updated_at)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.discover_promoted still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.discover_promoted FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_discover_promoted();
ALTER TABLE public.discover_promoted ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_discover_promoted()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('discover_promoted', jsonb_build_object('id', OLD.id), OLD.row_v, now());
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb BEFORE DELETE ON public.discover_promoted FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_discover_promoted();
CREATE OR REPLACE FUNCTION public.scd_erase_discover_promoted(k jsonb, upto bigint)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    DELETE FROM public.discover_promoted t WHERE t.id = (k ->> 'id')::uuid AND t.row_v <= upto;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
    IF EXISTS (SELECT 1 FROM public.discover_promoted t WHERE t.id = (k ->> 'id')::uuid) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('discover_promoted', k) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_discover_promoted'::regclass);
    END IF;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_tomb_check_discover_promoted()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'discover_promoted' AND x.pk = jsonb_build_object('id', NEW.id));
    IF tv IS NOT NULL AND NEW.row_v <= tv THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'discover_promoted', 'pk', jsonb_build_object('id', NEW.id), 'v', tv)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_discover_promoted'::regclass);
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.discover_promoted FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_discover_promoted();
ALTER TABLE public.discover_promoted ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_tomb_guard_discover_promoted()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN OLD;
    END IF;
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'discover_promoted' AND x.pk = jsonb_build_object('id', OLD.id));
    IF tv IS NULL OR OLD.row_v <= tv THEN
        RETURN OLD;
    END IF;
    INSERT INTO public.repl_resend (tbl, pk) VALUES ('discover_promoted', jsonb_build_object('id', OLD.id)) ON CONFLICT DO NOTHING;
    PERFORM nextval('public.scd_resend_discover_promoted'::regclass);
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_guard BEFORE DELETE ON public.discover_promoted FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_guard_discover_promoted();
ALTER TABLE public.discover_promoted ENABLE REPLICA TRIGGER zz_tomb_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_discover_promoted()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(greatest(NEW.row_v, (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'discover_promoted' AND x.pk = jsonb_build_object('id', NEW.id))));
        RETURN NEW;
    END IF;
    IF (NEW.active, NEW.created_at, NEW.entity_id, NEW.entity_type, NEW.note, NEW."position", NEW.updated_at) IS DISTINCT FROM (OLD.active, OLD.created_at, OLD.entity_id, OLD.entity_type, OLD.note, OLD."position", OLD.updated_at) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.discover_promoted FOR EACH ROW EXECUTE FUNCTION public.scd_ver_discover_promoted();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.discover_promoted FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.discover_promoted ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_merge_discover_settings()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.show_star := OLD.show_star;
            NEW.star_limit := OLD.star_limit;
            NEW.star_strategy := OLD.star_strategy;
            NEW.updated_at := OLD.updated_at;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.discover_settings t WHERE t.id = NEW.id;
        IF FOUND THEN
            IF NEW.row_v > o.row_v THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.discover_settings t SET
                    show_star = CASE WHEN NEW.row_v > o.row_v THEN NEW.show_star ELSE o.show_star END,
                    star_limit = CASE WHEN NEW.row_v > o.row_v THEN NEW.star_limit ELSE o.star_limit END,
                    star_strategy = CASE WHEN NEW.row_v > o.row_v THEN NEW.star_strategy ELSE o.star_strategy END,
                    updated_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.updated_at ELSE o.updated_at END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE t.id = o.id;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.discover_settings (id, row_v, show_star, star_limit, star_strategy, updated_at)
        VALUES (NEW.id, NEW.row_v, NEW.show_star, NEW.star_limit, NEW.star_strategy, NEW.updated_at)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.discover_settings still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.discover_settings FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_discover_settings();
ALTER TABLE public.discover_settings ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_ver_discover_settings()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.show_star, NEW.star_limit, NEW.star_strategy, NEW.updated_at) IS DISTINCT FROM (OLD.show_star, OLD.star_limit, OLD.star_strategy, OLD.updated_at) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.discover_settings FOR EACH ROW EXECUTE FUNCTION public.scd_ver_discover_settings();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.discover_settings FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.discover_settings ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.discover_tag_counts FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_discover_tag_counts', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_discover_tag_counts()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'discover_tag_counts' AND p.pk = jsonb_build_object('tag', OLD.tag)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.tag IS DISTINCT FROM OLD.tag THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'discover_tag_counts' AND p.pk = jsonb_build_object('tag', OLD.tag)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'discover_tag_counts' AND p.pk = jsonb_build_object('tag', NEW.tag)) THEN
                    INSERT INTO public.discover_tag_counts (artist_count, refreshed_at, tag) VALUES (NEW.artist_count, NEW.refreshed_at, NEW.tag);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'discover_tag_counts' AND p.pk = jsonb_build_object('tag', NEW.tag)) THEN
                DELETE FROM public.discover_tag_counts t WHERE t.tag = OLD.tag;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'discover_tag_counts' AND p.pk = jsonb_build_object('tag', NEW.tag)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.discover_tag_counts t WHERE t.tag = NEW.tag FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'discover_tag_counts' AND p.pk = jsonb_build_object('tag', NEW.tag)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.discover_tag_counts t SET artist_count = NEW.artist_count, refreshed_at = NEW.refreshed_at WHERE t.tag = NEW.tag;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.discover_tag_counts (artist_count, refreshed_at, tag)
        VALUES (NEW.artist_count, NEW.refreshed_at, NEW.tag)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.discover_tag_counts still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.discover_tag_counts FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_discover_tag_counts();
ALTER TABLE public.discover_tag_counts ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.discover_tag_counts FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.discover_tag_counts ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_merge_disliked_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.created_at := OLD.created_at;
            NEW.sc_track_id := OLD.sc_track_id;
            NEW.sc_user_id := OLD.sc_user_id;
            NEW.track_data := OLD.track_data;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.disliked_tracks t WHERE t.id = NEW.id;
        IF FOUND THEN
            IF NEW.row_v > o.row_v OR (((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) AND NEW.created_at < o.created_at) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.disliked_tracks t SET
                    created_at = CASE WHEN ((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.row_v > o.row_v THEN NEW.created_at ELSE o.created_at END,
                    sc_track_id = CASE WHEN NEW.row_v > o.row_v THEN NEW.sc_track_id ELSE o.sc_track_id END,
                    sc_user_id = CASE WHEN NEW.row_v > o.row_v THEN NEW.sc_user_id ELSE o.sc_user_id END,
                    track_data = CASE WHEN NEW.row_v > o.row_v THEN NEW.track_data ELSE o.track_data END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE t.id = o.id;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        SELECT t.* INTO o FROM public.disliked_tracks t WHERE (t.sc_user_id, t.sc_track_id) = (NEW.sc_user_id, NEW.sc_track_id);
        IF FOUND THEN
            PERFORM nextval('public.scd_conflict_disliked_tracks'::regclass);
            IF NEW.row_v > o.row_v OR (((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) AND NEW.created_at < o.created_at) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.disliked_tracks t SET
                    created_at = CASE WHEN ((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.row_v > o.row_v THEN NEW.created_at ELSE o.created_at END,
                    sc_track_id = CASE WHEN NEW.row_v > o.row_v THEN NEW.sc_track_id ELSE o.sc_track_id END,
                    sc_user_id = CASE WHEN NEW.row_v > o.row_v THEN NEW.sc_user_id ELSE o.sc_user_id END,
                    track_data = CASE WHEN NEW.row_v > o.row_v THEN NEW.track_data ELSE o.track_data END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE t.id = o.id;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'disliked_tracks' AND x.pk = jsonb_build_object('id', NEW.id));
        IF tv IS NOT NULL AND NEW.row_v <= tv THEN
            INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'disliked_tracks', 'pk', jsonb_build_object('id', NEW.id), 'v', tv)) ON CONFLICT DO NOTHING;
            PERFORM nextval('public.scd_resend_disliked_tracks'::regclass);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.disliked_tracks (created_at, id, row_v, sc_track_id, sc_user_id, track_data)
        VALUES (NEW.created_at, NEW.id, NEW.row_v, NEW.sc_track_id, NEW.sc_user_id, NEW.track_data)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.disliked_tracks still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.disliked_tracks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_disliked_tracks();
ALTER TABLE public.disliked_tracks ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_disliked_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('disliked_tracks', jsonb_build_object('id', OLD.id), OLD.row_v, now());
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb BEFORE DELETE ON public.disliked_tracks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_disliked_tracks();
CREATE OR REPLACE FUNCTION public.scd_erase_disliked_tracks(k jsonb, upto bigint)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    DELETE FROM public.disliked_tracks t WHERE t.id = (k ->> 'id')::uuid AND t.row_v <= upto;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
    IF EXISTS (SELECT 1 FROM public.disliked_tracks t WHERE t.id = (k ->> 'id')::uuid) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('disliked_tracks', k) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_disliked_tracks'::regclass);
    END IF;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_tomb_check_disliked_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'disliked_tracks' AND x.pk = jsonb_build_object('id', NEW.id));
    IF tv IS NOT NULL AND NEW.row_v <= tv THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'disliked_tracks', 'pk', jsonb_build_object('id', NEW.id), 'v', tv)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_disliked_tracks'::regclass);
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.disliked_tracks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_disliked_tracks();
ALTER TABLE public.disliked_tracks ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_tomb_guard_disliked_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN OLD;
    END IF;
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'disliked_tracks' AND x.pk = jsonb_build_object('id', OLD.id));
    IF tv IS NULL OR OLD.row_v <= tv THEN
        RETURN OLD;
    END IF;
    INSERT INTO public.repl_resend (tbl, pk) VALUES ('disliked_tracks', jsonb_build_object('id', OLD.id)) ON CONFLICT DO NOTHING;
    PERFORM nextval('public.scd_resend_disliked_tracks'::regclass);
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_guard BEFORE DELETE ON public.disliked_tracks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_guard_disliked_tracks();
ALTER TABLE public.disliked_tracks ENABLE REPLICA TRIGGER zz_tomb_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_disliked_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(greatest(NEW.row_v, (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'disliked_tracks' AND x.pk = jsonb_build_object('id', NEW.id))));
        RETURN NEW;
    END IF;
    IF (NEW.created_at, NEW.sc_track_id, NEW.sc_user_id, NEW.track_data) IS DISTINCT FROM (OLD.created_at, OLD.sc_track_id, OLD.sc_user_id, OLD.track_data) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.disliked_tracks FOR EACH ROW EXECUTE FUNCTION public.scd_ver_disliked_tracks();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.disliked_tracks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.disliked_tracks ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.enrich_calibration FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_enrich_calibration', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_max_entity_reads()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    n bigint;
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF current_setting('scd.merging', true) = 'on' THEN
            RETURN NEW;
        END IF;
        FOR attempt IN 1..3 LOOP
            UPDATE public.entity_reads t SET last_read_at = NEW.last_read_at WHERE t.entity_urn = NEW.entity_urn AND (t.last_read_at IS NULL OR t.last_read_at < NEW.last_read_at);
            IF FOUND THEN
                RETURN NULL;
            END IF;
            PERFORM 1 FROM public.entity_reads t WHERE t.entity_urn = NEW.entity_urn;
            IF FOUND THEN
                RETURN NULL;
            END IF;
            PERFORM pg_catalog.set_config('scd.merging', 'on', true);
            INSERT INTO public.entity_reads (entity_urn, last_read_at)
            VALUES (NEW.entity_urn, NEW.last_read_at)
            ON CONFLICT DO NOTHING;
            GET DIAGNOSTICS n = ROW_COUNT;
            PERFORM pg_catalog.set_config('scd.merging', '', true);
            IF n > 0 THEN
                RETURN NULL;
            END IF;
        END LOOP;
        RAISE EXCEPTION 'scd: insert into public.entity_reads still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
    END IF;
    IF OLD.last_read_at IS NOT NULL AND (NEW.last_read_at IS NULL OR NEW.last_read_at <= OLD.last_read_at) THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_max BEFORE INSERT OR UPDATE ON public.entity_reads FOR EACH ROW WHEN (public.scd_role() = 'core' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_max_entity_reads();
ALTER TABLE public.entity_reads ENABLE REPLICA TRIGGER zz_max;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.entity_reads FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.entity_reads ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_merge_featured_items()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.active := OLD.active;
            NEW.created_at := OLD.created_at;
            NEW.sc_urn := OLD.sc_urn;
            NEW.type := OLD.type;
            NEW.weight := OLD.weight;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.featured_items t WHERE t.id = NEW.id;
        IF FOUND THEN
            IF NEW.row_v > o.row_v OR (((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) AND NEW.created_at < o.created_at) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.featured_items t SET
                    active = CASE WHEN NEW.row_v > o.row_v THEN NEW.active ELSE o.active END,
                    created_at = CASE WHEN ((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.row_v > o.row_v THEN NEW.created_at ELSE o.created_at END,
                    sc_urn = CASE WHEN NEW.row_v > o.row_v THEN NEW.sc_urn ELSE o.sc_urn END,
                    type = CASE WHEN NEW.row_v > o.row_v THEN NEW.type ELSE o.type END,
                    weight = CASE WHEN NEW.row_v > o.row_v THEN NEW.weight ELSE o.weight END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE t.id = o.id;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'featured_items' AND x.pk = jsonb_build_object('id', NEW.id));
        IF tv IS NOT NULL THEN
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.featured_items (active, created_at, id, row_v, sc_urn, type, weight)
        VALUES (NEW.active, NEW.created_at, NEW.id, NEW.row_v, NEW.sc_urn, NEW.type, NEW.weight)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.featured_items still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.featured_items FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_featured_items();
ALTER TABLE public.featured_items ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_featured_items()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('featured_items', jsonb_build_object('id', OLD.id), OLD.row_v, now());
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb BEFORE DELETE ON public.featured_items FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_featured_items();
CREATE OR REPLACE FUNCTION public.scd_erase_featured_items(k jsonb, upto bigint)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    DELETE FROM public.featured_items t WHERE t.id = (k ->> 'id')::uuid;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_tomb_check_featured_items()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'featured_items' AND x.pk = jsonb_build_object('id', NEW.id));
    IF tv IS NOT NULL THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.featured_items FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_featured_items();
ALTER TABLE public.featured_items ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_ver_featured_items()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.active, NEW.created_at, NEW.sc_urn, NEW.type, NEW.weight) IS DISTINCT FROM (OLD.active, OLD.created_at, OLD.sc_urn, OLD.type, OLD.weight) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.featured_items FOR EACH ROW EXECUTE FUNCTION public.scd_ver_featured_items();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.featured_items FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.featured_items ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.job_status FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_job_status', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_job_status()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'job_status' AND p.pk = jsonb_build_object('kind', OLD.kind, 'dedup_key', OLD.dedup_key)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.kind, NEW.dedup_key) IS DISTINCT FROM (OLD.kind, OLD.dedup_key) THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'job_status' AND p.pk = jsonb_build_object('kind', OLD.kind, 'dedup_key', OLD.dedup_key)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'job_status' AND p.pk = jsonb_build_object('kind', NEW.kind, 'dedup_key', NEW.dedup_key)) THEN
                    INSERT INTO public.job_status (available_at, dedup_key, kind, state, updated_at) VALUES (NEW.available_at, NEW.dedup_key, NEW.kind, NEW.state, NEW.updated_at);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'job_status' AND p.pk = jsonb_build_object('kind', NEW.kind, 'dedup_key', NEW.dedup_key)) THEN
                DELETE FROM public.job_status t WHERE (t.kind, t.dedup_key) = (OLD.kind, OLD.dedup_key);
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'job_status' AND p.pk = jsonb_build_object('kind', NEW.kind, 'dedup_key', NEW.dedup_key)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.job_status t WHERE (t.kind, t.dedup_key) = (NEW.kind, NEW.dedup_key) FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'job_status' AND p.pk = jsonb_build_object('kind', NEW.kind, 'dedup_key', NEW.dedup_key)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.job_status t SET available_at = NEW.available_at, state = NEW.state, updated_at = NEW.updated_at WHERE (t.kind, t.dedup_key) = (NEW.kind, NEW.dedup_key);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.job_status (available_at, dedup_key, kind, state, updated_at)
        VALUES (NEW.available_at, NEW.dedup_key, NEW.kind, NEW.state, NEW.updated_at)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.job_status still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.job_status FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_job_status();
ALTER TABLE public.job_status ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.job_status FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.job_status ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_merge_link_requests()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.claim_token := OLD.claim_token;
            NEW.created_at := OLD.created_at;
            NEW.error := OLD.error;
            NEW.expires_at := OLD.expires_at;
            NEW.mode := OLD.mode;
            NEW.source_session_id := OLD.source_session_id;
            NEW.status := OLD.status;
            NEW.target_session_id := OLD.target_session_id;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.link_requests t WHERE t.id = NEW.id;
        IF FOUND THEN
            IF NEW.row_v > o.row_v OR (((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) AND NEW.created_at < o.created_at) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.link_requests t SET
                    claim_token = CASE WHEN NEW.row_v > o.row_v THEN NEW.claim_token ELSE o.claim_token END,
                    created_at = CASE WHEN ((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.row_v > o.row_v THEN NEW.created_at ELSE o.created_at END,
                    error = CASE WHEN NEW.row_v > o.row_v THEN NEW.error ELSE o.error END,
                    expires_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.expires_at ELSE o.expires_at END,
                    mode = CASE WHEN NEW.row_v > o.row_v THEN NEW.mode ELSE o.mode END,
                    source_session_id = CASE WHEN NEW.row_v > o.row_v THEN NEW.source_session_id ELSE o.source_session_id END,
                    status = CASE WHEN NEW.row_v > o.row_v THEN NEW.status ELSE o.status END,
                    target_session_id = CASE WHEN NEW.row_v > o.row_v THEN NEW.target_session_id ELSE o.target_session_id END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE t.id = o.id;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'link_requests' AND x.pk = jsonb_build_object('id', NEW.id));
        IF tv IS NOT NULL THEN
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.link_requests (claim_token, created_at, error, expires_at, id, mode, row_v, source_session_id, status, target_session_id)
        VALUES (NEW.claim_token, NEW.created_at, NEW.error, NEW.expires_at, NEW.id, NEW.mode, NEW.row_v, NEW.source_session_id, NEW.status, NEW.target_session_id)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.link_requests still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.link_requests FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_link_requests();
ALTER TABLE public.link_requests ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_link_requests()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('link_requests', jsonb_build_object('id', OLD.id), OLD.row_v, now());
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb BEFORE DELETE ON public.link_requests FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_link_requests();
CREATE OR REPLACE FUNCTION public.scd_erase_link_requests(k jsonb, upto bigint)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    DELETE FROM public.link_requests t WHERE t.id = (k ->> 'id')::uuid;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_tomb_check_link_requests()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'link_requests' AND x.pk = jsonb_build_object('id', NEW.id));
    IF tv IS NOT NULL THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.link_requests FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_link_requests();
ALTER TABLE public.link_requests ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_ver_link_requests()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.claim_token, NEW.created_at, NEW.error, NEW.expires_at, NEW.mode, NEW.source_session_id, NEW.status, NEW.target_session_id) IS DISTINCT FROM (OLD.claim_token, OLD.created_at, OLD.error, OLD.expires_at, OLD.mode, OLD.source_session_id, OLD.status, OLD.target_session_id) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.link_requests FOR EACH ROW EXECUTE FUNCTION public.scd_ver_link_requests();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.link_requests FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.link_requests ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_skip_listening_history()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    PERFORM 1 FROM public.listening_history t WHERE t.id = NEW.id;
    IF FOUND THEN
        RETURN NULL;
    END IF;
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'listening_history' AND x.pk = jsonb_build_object('id', NEW.id));
    IF tv IS NOT NULL THEN
        RETURN NULL;
    END IF;
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    INSERT INTO public.listening_history (artist_name, artist_urn, artwork_url, duration, id, played_at, sc_track_id, soundcloud_user_id, title)
    VALUES (NEW.artist_name, NEW.artist_urn, NEW.artwork_url, NEW.duration, NEW.id, NEW.played_at, NEW.sc_track_id, NEW.soundcloud_user_id, NEW.title)
    ON CONFLICT DO NOTHING;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_skip BEFORE INSERT ON public.listening_history FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_skip_listening_history();
ALTER TABLE public.listening_history ENABLE REPLICA TRIGGER zz_skip;

CREATE OR REPLACE FUNCTION public.scd_tomb_listening_history()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('listening_history', jsonb_build_object('id', OLD.id), 0, now());
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb BEFORE DELETE ON public.listening_history FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_listening_history();
CREATE OR REPLACE FUNCTION public.scd_erase_listening_history(k jsonb, upto bigint)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    DELETE FROM public.listening_history t WHERE t.id = (k ->> 'id')::uuid;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_tomb_check_listening_history()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'listening_history' AND x.pk = jsonb_build_object('id', NEW.id));
    IF tv IS NOT NULL THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.listening_history FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_listening_history();
ALTER TABLE public.listening_history ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.listening_history FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.listening_history ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_merge_login_requests()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.code_verifier := OLD.code_verifier;
            NEW.created_at := OLD.created_at;
            NEW.error := OLD.error;
            NEW.expires_at := OLD.expires_at;
            NEW.oauth_app_id := OLD.oauth_app_id;
            NEW.profile_ok := OLD.profile_ok;
            NEW.redirect_url := OLD.redirect_url;
            NEW.result_session_id := OLD.result_session_id;
            NEW.retry_count := OLD.retry_count;
            NEW.state := OLD.state;
            NEW.status := OLD.status;
            NEW.step := OLD.step;
            NEW.target_session_id := OLD.target_session_id;
            NEW.username := OLD.username;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.login_requests t WHERE t.id = NEW.id;
        IF FOUND THEN
            IF NEW.row_v > o.row_v OR (((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) AND NEW.created_at < o.created_at) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.login_requests t SET
                    code_verifier = CASE WHEN NEW.row_v > o.row_v THEN NEW.code_verifier ELSE o.code_verifier END,
                    created_at = CASE WHEN ((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.row_v > o.row_v THEN NEW.created_at ELSE o.created_at END,
                    error = CASE WHEN NEW.row_v > o.row_v THEN NEW.error ELSE o.error END,
                    expires_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.expires_at ELSE o.expires_at END,
                    oauth_app_id = CASE WHEN NEW.row_v > o.row_v THEN NEW.oauth_app_id ELSE o.oauth_app_id END,
                    profile_ok = CASE WHEN NEW.row_v > o.row_v THEN NEW.profile_ok ELSE o.profile_ok END,
                    redirect_url = CASE WHEN NEW.row_v > o.row_v THEN NEW.redirect_url ELSE o.redirect_url END,
                    result_session_id = CASE WHEN NEW.row_v > o.row_v THEN NEW.result_session_id ELSE o.result_session_id END,
                    retry_count = CASE WHEN NEW.row_v > o.row_v THEN NEW.retry_count ELSE o.retry_count END,
                    state = CASE WHEN NEW.row_v > o.row_v THEN NEW.state ELSE o.state END,
                    status = CASE WHEN NEW.row_v > o.row_v THEN NEW.status ELSE o.status END,
                    step = CASE WHEN NEW.row_v > o.row_v THEN NEW.step ELSE o.step END,
                    target_session_id = CASE WHEN NEW.row_v > o.row_v THEN NEW.target_session_id ELSE o.target_session_id END,
                    username = CASE WHEN NEW.row_v > o.row_v THEN NEW.username ELSE o.username END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE t.id = o.id;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'login_requests' AND x.pk = jsonb_build_object('id', NEW.id));
        IF tv IS NOT NULL THEN
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.login_requests (code_verifier, created_at, error, expires_at, id, oauth_app_id, profile_ok, redirect_url, result_session_id, retry_count, row_v, state, status, step, target_session_id, username)
        VALUES (NEW.code_verifier, NEW.created_at, NEW.error, NEW.expires_at, NEW.id, NEW.oauth_app_id, NEW.profile_ok, NEW.redirect_url, NEW.result_session_id, NEW.retry_count, NEW.row_v, NEW.state, NEW.status, NEW.step, NEW.target_session_id, NEW.username)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.login_requests still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.login_requests FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_login_requests();
ALTER TABLE public.login_requests ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_login_requests()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('login_requests', jsonb_build_object('id', OLD.id), OLD.row_v, now());
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb BEFORE DELETE ON public.login_requests FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_login_requests();
CREATE OR REPLACE FUNCTION public.scd_erase_login_requests(k jsonb, upto bigint)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    DELETE FROM public.login_requests t WHERE t.id = (k ->> 'id')::uuid;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_tomb_check_login_requests()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'login_requests' AND x.pk = jsonb_build_object('id', NEW.id));
    IF tv IS NOT NULL THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.login_requests FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_login_requests();
ALTER TABLE public.login_requests ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_ver_login_requests()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.code_verifier, NEW.created_at, NEW.error, NEW.expires_at, NEW.oauth_app_id, NEW.profile_ok, NEW.redirect_url, NEW.result_session_id, NEW.retry_count, NEW.state, NEW.status, NEW.step, NEW.target_session_id, NEW.username) IS DISTINCT FROM (OLD.code_verifier, OLD.created_at, OLD.error, OLD.expires_at, OLD.oauth_app_id, OLD.profile_ok, OLD.redirect_url, OLD.result_session_id, OLD.retry_count, OLD.state, OLD.status, OLD.step, OLD.target_session_id, OLD.username) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.login_requests FOR EACH ROW EXECUTE FUNCTION public.scd_ver_login_requests();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.login_requests FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.login_requests ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_core_col_lyrics_cache()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF NEW.fts IS DISTINCT FROM OLD.fts THEN
        NEW.fts := OLD.fts;
        PERFORM nextval('public.scd_core_col_lyrics_cache'::regclass);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_core_col BEFORE UPDATE OF fts ON public.lyrics_cache FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_core_col_lyrics_cache();

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.lyrics_cache FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_lyrics_cache', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_lyrics_cache()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'lyrics_cache' AND p.pk = jsonb_build_object('sc_track_id', OLD.sc_track_id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.sc_track_id IS DISTINCT FROM OLD.sc_track_id THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'lyrics_cache' AND p.pk = jsonb_build_object('sc_track_id', OLD.sc_track_id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'lyrics_cache' AND p.pk = jsonb_build_object('sc_track_id', NEW.sc_track_id)) THEN
                    INSERT INTO public.lyrics_cache (content_generation, content_hash, created_at, embedded_at, embedding_state, fts, language, language_confidence, plain_source, plain_text, sc_track_id, source, synced_lrc, synced_source, synced_version, track_id, updated_at) VALUES (NEW.content_generation, NEW.content_hash, NEW.created_at, NEW.embedded_at, NEW.embedding_state, NEW.fts, NEW.language, NEW.language_confidence, NEW.plain_source, NEW.plain_text, NEW.sc_track_id, NEW.source, NEW.synced_lrc, NEW.synced_source, NEW.synced_version, NEW.track_id, NEW.updated_at);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'lyrics_cache' AND p.pk = jsonb_build_object('sc_track_id', NEW.sc_track_id)) THEN
                DELETE FROM public.lyrics_cache t WHERE t.sc_track_id = OLD.sc_track_id;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'lyrics_cache' AND p.pk = jsonb_build_object('sc_track_id', NEW.sc_track_id)) THEN
                IF NEW.fts IS DISTINCT FROM OLD.fts THEN
                    NEW.content_generation := OLD.content_generation;
                    NEW.content_hash := OLD.content_hash;
                    NEW.created_at := OLD.created_at;
                    NEW.embedded_at := OLD.embedded_at;
                    NEW.embedding_state := OLD.embedding_state;
                    NEW.language := OLD.language;
                    NEW.language_confidence := OLD.language_confidence;
                    NEW.plain_source := OLD.plain_source;
                    NEW.plain_text := OLD.plain_text;
                    NEW.source := OLD.source;
                    NEW.synced_lrc := OLD.synced_lrc;
                    NEW.synced_source := OLD.synced_source;
                    NEW.synced_version := OLD.synced_version;
                    NEW.track_id := OLD.track_id;
                    NEW.updated_at := OLD.updated_at;
                    RETURN NEW;
                END IF;
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.lyrics_cache t WHERE t.sc_track_id = NEW.sc_track_id FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'lyrics_cache' AND p.pk = jsonb_build_object('sc_track_id', NEW.sc_track_id)) THEN
            PERFORM pg_catalog.set_config('scd.merging', 'on', true);
            UPDATE public.lyrics_cache t SET fts = NEW.fts WHERE t.sc_track_id = NEW.sc_track_id AND t.fts IS DISTINCT FROM NEW.fts;
            PERFORM pg_catalog.set_config('scd.merging', '', true);
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.lyrics_cache t SET content_generation = NEW.content_generation, content_hash = NEW.content_hash, created_at = NEW.created_at, embedded_at = NEW.embedded_at, embedding_state = NEW.embedding_state, fts = NEW.fts, language = NEW.language, language_confidence = NEW.language_confidence, plain_source = NEW.plain_source, plain_text = NEW.plain_text, source = NEW.source, synced_lrc = NEW.synced_lrc, synced_source = NEW.synced_source, synced_version = NEW.synced_version, track_id = NEW.track_id, updated_at = NEW.updated_at WHERE t.sc_track_id = NEW.sc_track_id;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.lyrics_cache (content_generation, content_hash, created_at, embedded_at, embedding_state, fts, language, language_confidence, plain_source, plain_text, sc_track_id, source, synced_lrc, synced_source, synced_version, track_id, updated_at)
        VALUES (NEW.content_generation, NEW.content_hash, NEW.created_at, NEW.embedded_at, NEW.embedding_state, NEW.fts, NEW.language, NEW.language_confidence, NEW.plain_source, NEW.plain_text, NEW.sc_track_id, NEW.source, NEW.synced_lrc, NEW.synced_source, NEW.synced_version, NEW.track_id, NEW.updated_at)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.lyrics_cache still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.lyrics_cache FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_lyrics_cache();
ALTER TABLE public.lyrics_cache ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.lyrics_cache FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.lyrics_cache ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.lyrics_embedding_wire_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_lyrics_embedding_wire_state', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.lyrics_lookup_backfill_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_lyrics_lookup_backfill_state', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.lyrics_lookup_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_lyrics_lookup_state', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_lyrics_lookup_state()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
    k0 boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'lyrics_lookup_state' AND p.pk = jsonb_build_object('track_id', OLD.track_id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.track_id IS DISTINCT FROM OLD.track_id THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'lyrics_lookup_state' AND p.pk = jsonb_build_object('track_id', OLD.track_id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'lyrics_lookup_state' AND p.pk = jsonb_build_object('track_id', NEW.track_id)) THEN
                    INSERT INTO public.lyrics_lookup_state (algorithm_version, attempts, claim_expires_at, claim_job_generation, claim_job_id, claim_job_lease_id, claim_state_generation, created_at, failure_streak, generation, input_artist, input_duration_ms, input_genius_song_id, input_genius_url, input_release_date, input_sc_created_at, input_title, last_attempt_at, last_error, last_outcome, miss_streak, next_run_at, priority, retry_after_at, sc_track_id, source_set_version, status, track_id, updated_at, wake_durable_at, wake_generation, wake_message_id) VALUES (NEW.algorithm_version, NEW.attempts, NEW.claim_expires_at, NEW.claim_job_generation, NEW.claim_job_id, NEW.claim_job_lease_id, NEW.claim_state_generation, NEW.created_at, NEW.failure_streak, NEW.generation, NEW.input_artist, NEW.input_duration_ms, NEW.input_genius_song_id, NEW.input_genius_url, NEW.input_release_date, NEW.input_sc_created_at, NEW.input_title, NEW.last_attempt_at, NEW.last_error, NEW.last_outcome, NEW.miss_streak, NEW.next_run_at, NEW.priority, NEW.retry_after_at, NEW.sc_track_id, NEW.source_set_version, NEW.status, NEW.track_id, NEW.updated_at, NEW.wake_durable_at, NEW.wake_generation, NEW.wake_message_id);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'lyrics_lookup_state' AND p.pk = jsonb_build_object('track_id', NEW.track_id)) THEN
                DELETE FROM public.lyrics_lookup_state t WHERE t.track_id = OLD.track_id;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'lyrics_lookup_state' AND p.pk = jsonb_build_object('track_id', NEW.track_id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        k0 := NEW.sc_track_id IS DISTINCT FROM OLD.sc_track_id;
        IF k0 THEN
            FOR r IN SELECT t.track_id FROM public.lyrics_lookup_state t WHERE t.sc_track_id = NEW.sc_track_id AND t.track_id IS DISTINCT FROM NEW.track_id LOOP
                PERFORM public.scd_purge_lyrics_lookup_state(jsonb_build_object('track_id', r.track_id));
            END LOOP;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.lyrics_lookup_state t WHERE t.track_id = NEW.track_id FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'lyrics_lookup_state' AND p.pk = jsonb_build_object('track_id', NEW.track_id)) THEN
            RETURN NULL;
        END IF;
        FOR r IN SELECT t.track_id FROM public.lyrics_lookup_state t WHERE t.sc_track_id = NEW.sc_track_id AND t.track_id IS DISTINCT FROM NEW.track_id LOOP
            PERFORM public.scd_purge_lyrics_lookup_state(jsonb_build_object('track_id', r.track_id));
        END LOOP;
        IF held THEN
            UPDATE public.lyrics_lookup_state t SET algorithm_version = NEW.algorithm_version, attempts = NEW.attempts, claim_expires_at = NEW.claim_expires_at, claim_job_generation = NEW.claim_job_generation, claim_job_id = NEW.claim_job_id, claim_job_lease_id = NEW.claim_job_lease_id, claim_state_generation = NEW.claim_state_generation, created_at = NEW.created_at, failure_streak = NEW.failure_streak, generation = NEW.generation, input_artist = NEW.input_artist, input_duration_ms = NEW.input_duration_ms, input_genius_song_id = NEW.input_genius_song_id, input_genius_url = NEW.input_genius_url, input_release_date = NEW.input_release_date, input_sc_created_at = NEW.input_sc_created_at, input_title = NEW.input_title, last_attempt_at = NEW.last_attempt_at, last_error = NEW.last_error, last_outcome = NEW.last_outcome, miss_streak = NEW.miss_streak, next_run_at = NEW.next_run_at, priority = NEW.priority, retry_after_at = NEW.retry_after_at, sc_track_id = NEW.sc_track_id, source_set_version = NEW.source_set_version, status = NEW.status, updated_at = NEW.updated_at, wake_durable_at = NEW.wake_durable_at, wake_generation = NEW.wake_generation, wake_message_id = NEW.wake_message_id WHERE t.track_id = NEW.track_id;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.lyrics_lookup_state (algorithm_version, attempts, claim_expires_at, claim_job_generation, claim_job_id, claim_job_lease_id, claim_state_generation, created_at, failure_streak, generation, input_artist, input_duration_ms, input_genius_song_id, input_genius_url, input_release_date, input_sc_created_at, input_title, last_attempt_at, last_error, last_outcome, miss_streak, next_run_at, priority, retry_after_at, sc_track_id, source_set_version, status, track_id, updated_at, wake_durable_at, wake_generation, wake_message_id)
        VALUES (NEW.algorithm_version, NEW.attempts, NEW.claim_expires_at, NEW.claim_job_generation, NEW.claim_job_id, NEW.claim_job_lease_id, NEW.claim_state_generation, NEW.created_at, NEW.failure_streak, NEW.generation, NEW.input_artist, NEW.input_duration_ms, NEW.input_genius_song_id, NEW.input_genius_url, NEW.input_release_date, NEW.input_sc_created_at, NEW.input_title, NEW.last_attempt_at, NEW.last_error, NEW.last_outcome, NEW.miss_streak, NEW.next_run_at, NEW.priority, NEW.retry_after_at, NEW.sc_track_id, NEW.source_set_version, NEW.status, NEW.track_id, NEW.updated_at, NEW.wake_durable_at, NEW.wake_generation, NEW.wake_message_id)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.lyrics_lookup_state still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.lyrics_lookup_state FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_lyrics_lookup_state();
ALTER TABLE public.lyrics_lookup_state ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.lyrics_lookup_state FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.lyrics_lookup_state ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_merge_oauth_app_request_cooldowns()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.failure_count := OLD.failure_count;
            NEW.retry_at := OLD.retry_at;
            NEW.updated_at := OLD.updated_at;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.oauth_app_request_cooldowns t WHERE t.oauth_app_id = NEW.oauth_app_id;
        IF FOUND THEN
            IF NEW.row_v > o.row_v THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.oauth_app_request_cooldowns t SET
                    failure_count = CASE WHEN NEW.row_v > o.row_v THEN NEW.failure_count ELSE o.failure_count END,
                    retry_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.retry_at ELSE o.retry_at END,
                    updated_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.updated_at ELSE o.updated_at END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE t.oauth_app_id = o.oauth_app_id;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.oauth_app_request_cooldowns (failure_count, oauth_app_id, retry_at, row_v, updated_at)
        VALUES (NEW.failure_count, NEW.oauth_app_id, NEW.retry_at, NEW.row_v, NEW.updated_at)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.oauth_app_request_cooldowns still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.oauth_app_request_cooldowns FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_oauth_app_request_cooldowns();
ALTER TABLE public.oauth_app_request_cooldowns ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_orphan_oauth_app_request_cooldowns()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF NEW.oauth_app_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM public.oauth_apps p WHERE p.id = NEW.oauth_app_id) THEN
        PERFORM nextval('public.scd_orphan_oauth_app_request_cooldowns'::regclass);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_orphan BEFORE INSERT OR UPDATE OF oauth_app_id ON public.oauth_app_request_cooldowns FOR EACH ROW WHEN (public.scd_role() = 'core' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_orphan_oauth_app_request_cooldowns();
ALTER TABLE public.oauth_app_request_cooldowns ENABLE REPLICA TRIGGER zz_orphan;

CREATE OR REPLACE FUNCTION public.scd_ver_oauth_app_request_cooldowns()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.failure_count, NEW.retry_at, NEW.updated_at) IS DISTINCT FROM (OLD.failure_count, OLD.retry_at, OLD.updated_at) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.oauth_app_request_cooldowns FOR EACH ROW EXECUTE FUNCTION public.scd_ver_oauth_app_request_cooldowns();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.oauth_app_request_cooldowns FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.oauth_app_request_cooldowns ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.oauth_app_token_issuance_reservations FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_oauth_app_token_issuance_reservations', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.oauth_app_token_refresh_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_oauth_app_token_refresh_state', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.oauth_app_tokens FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_oauth_app_tokens', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_oauth_app_tokens()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'oauth_app_tokens' AND p.pk = jsonb_build_object('oauth_app_id', OLD.oauth_app_id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.oauth_app_id IS DISTINCT FROM OLD.oauth_app_id THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'oauth_app_tokens' AND p.pk = jsonb_build_object('oauth_app_id', OLD.oauth_app_id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'oauth_app_tokens' AND p.pk = jsonb_build_object('oauth_app_id', NEW.oauth_app_id)) THEN
                    INSERT INTO public.oauth_app_tokens (access_token, expires_at, generation, last_refresh_error, last_used_at, oauth_app_id, refresh_attempts, refresh_token, refreshed_at, scope) VALUES (NEW.access_token, NEW.expires_at, NEW.generation, NEW.last_refresh_error, NEW.last_used_at, NEW.oauth_app_id, NEW.refresh_attempts, NEW.refresh_token, NEW.refreshed_at, NEW.scope);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'oauth_app_tokens' AND p.pk = jsonb_build_object('oauth_app_id', NEW.oauth_app_id)) THEN
                DELETE FROM public.oauth_app_tokens t WHERE t.oauth_app_id = OLD.oauth_app_id;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'oauth_app_tokens' AND p.pk = jsonb_build_object('oauth_app_id', NEW.oauth_app_id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.oauth_app_tokens t WHERE t.oauth_app_id = NEW.oauth_app_id FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'oauth_app_tokens' AND p.pk = jsonb_build_object('oauth_app_id', NEW.oauth_app_id)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.oauth_app_tokens t SET access_token = NEW.access_token, expires_at = NEW.expires_at, generation = NEW.generation, last_refresh_error = NEW.last_refresh_error, last_used_at = NEW.last_used_at, refresh_attempts = NEW.refresh_attempts, refresh_token = NEW.refresh_token, refreshed_at = NEW.refreshed_at, scope = NEW.scope WHERE t.oauth_app_id = NEW.oauth_app_id;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.oauth_app_tokens (access_token, expires_at, generation, last_refresh_error, last_used_at, oauth_app_id, refresh_attempts, refresh_token, refreshed_at, scope)
        VALUES (NEW.access_token, NEW.expires_at, NEW.generation, NEW.last_refresh_error, NEW.last_used_at, NEW.oauth_app_id, NEW.refresh_attempts, NEW.refresh_token, NEW.refreshed_at, NEW.scope)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.oauth_app_tokens still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.oauth_app_tokens FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_oauth_app_tokens();
ALTER TABLE public.oauth_app_tokens ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.oauth_app_tokens FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.oauth_app_tokens ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_fk_oauth_apps()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF EXISTS (SELECT 1 FROM public.soundcloud_connections c WHERE c.oauth_app_id = OLD.id) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('oauth_apps', jsonb_build_object('id', OLD.id)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_restrict_oauth_apps'::regclass);
        RETURN NULL;
    END IF;
    DELETE FROM public.oauth_app_request_cooldowns c WHERE c.oauth_app_id = OLD.id;
    UPDATE public.oauth_app_token_issuance_reservations c SET oauth_app_id = NULL WHERE c.oauth_app_id = OLD.id;
    DELETE FROM public.oauth_app_token_refresh_state c WHERE c.oauth_app_id = OLD.id;
    DELETE FROM public.oauth_app_tokens c WHERE c.oauth_app_id = OLD.id;
    DELETE FROM public.sync_queue_action_pauses c WHERE c.oauth_app_id = OLD.id;
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_fk BEFORE DELETE ON public.oauth_apps FOR EACH ROW WHEN (public.scd_role() = 'core' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_fk_oauth_apps();
ALTER TABLE public.oauth_apps ENABLE REPLICA TRIGGER zz_fk;

CREATE OR REPLACE FUNCTION public.scd_merge_oauth_apps()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.active := OLD.active;
            NEW.client_id := OLD.client_id;
            NEW.client_secret := OLD.client_secret;
            NEW.created_at := OLD.created_at;
            NEW.name := OLD.name;
            NEW.redirect_uri := OLD.redirect_uri;
            NEW.updated_at := OLD.updated_at;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.oauth_apps t WHERE t.id = NEW.id;
        IF FOUND THEN
            IF NEW.row_v > o.row_v OR (((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) AND NEW.created_at < o.created_at) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.oauth_apps t SET
                    active = CASE WHEN NEW.row_v > o.row_v THEN NEW.active ELSE o.active END,
                    client_id = CASE WHEN NEW.row_v > o.row_v THEN NEW.client_id ELSE o.client_id END,
                    client_secret = CASE WHEN NEW.row_v > o.row_v THEN NEW.client_secret ELSE o.client_secret END,
                    created_at = CASE WHEN ((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.row_v > o.row_v THEN NEW.created_at ELSE o.created_at END,
                    name = CASE WHEN NEW.row_v > o.row_v THEN NEW.name ELSE o.name END,
                    redirect_uri = CASE WHEN NEW.row_v > o.row_v THEN NEW.redirect_uri ELSE o.redirect_uri END,
                    updated_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.updated_at ELSE o.updated_at END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE t.id = o.id;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        SELECT t.* INTO o FROM public.oauth_apps t WHERE (btrim(t.client_id, E'\u0009\u000A\u000B\u000C\u000D \u0085\u00A0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200A\u2028\u2029\u202F\u205F\u3000'::text)) = (btrim(NEW.client_id, E'\u0009\u000A\u000B\u000C\u000D \u0085\u00A0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200A\u2028\u2029\u202F\u205F\u3000'::text));
        IF FOUND THEN
            PERFORM nextval('public.scd_conflict_oauth_apps'::regclass);
            IF NEW.row_v > o.row_v OR (((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) AND NEW.created_at < o.created_at) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.oauth_apps t SET
                    active = CASE WHEN NEW.row_v > o.row_v THEN NEW.active ELSE o.active END,
                    client_id = CASE WHEN NEW.row_v > o.row_v THEN NEW.client_id ELSE o.client_id END,
                    client_secret = CASE WHEN NEW.row_v > o.row_v THEN NEW.client_secret ELSE o.client_secret END,
                    created_at = CASE WHEN ((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.row_v > o.row_v THEN NEW.created_at ELSE o.created_at END,
                    name = CASE WHEN NEW.row_v > o.row_v THEN NEW.name ELSE o.name END,
                    redirect_uri = CASE WHEN NEW.row_v > o.row_v THEN NEW.redirect_uri ELSE o.redirect_uri END,
                    updated_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.updated_at ELSE o.updated_at END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE t.id = o.id;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.oauth_apps (active, client_id, client_secret, created_at, id, last_used_at, name, redirect_uri, row_v, updated_at)
        VALUES (NEW.active, NEW.client_id, NEW.client_secret, NEW.created_at, NEW.id, NEW.last_used_at, NEW.name, NEW.redirect_uri, NEW.row_v, NEW.updated_at)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.oauth_apps still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.oauth_apps FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_oauth_apps();
ALTER TABLE public.oauth_apps ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_ver_oauth_apps()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.active, NEW.client_id, NEW.client_secret, NEW.created_at, NEW.name, NEW.redirect_uri, NEW.updated_at) IS DISTINCT FROM (OLD.active, OLD.client_id, OLD.client_secret, OLD.created_at, OLD.name, OLD.redirect_uri, OLD.updated_at) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.oauth_apps FOR EACH ROW EXECUTE FUNCTION public.scd_ver_oauth_apps();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.oauth_apps FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.oauth_apps ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.pipeline_event_receipts FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_pipeline_event_receipts', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlist_legacy_membership_intents FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlist_legacy_membership_intents', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_playlist_legacy_membership_intents()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
    k0 boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_legacy_membership_intents' AND p.pk = jsonb_build_object('archive_id', OLD.archive_id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.archive_id IS DISTINCT FROM OLD.archive_id THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_legacy_membership_intents' AND p.pk = jsonb_build_object('archive_id', OLD.archive_id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_legacy_membership_intents' AND p.pk = jsonb_build_object('archive_id', NEW.archive_id)) THEN
                    INSERT INTO public.playlist_legacy_membership_intents (archive_id, archived_at, classification, legacy_desired_revision, legacy_synced_revision, legacy_user_id, local_snapshot_id, playlist_urn, prior_classification, queue_created_at, queue_failed_at, queue_generation, queue_id, queue_last_error, queue_next_run_at, queue_payload, queue_retry_count, remote_attempted_generation, remote_completed_generation, remote_result, resolved_at, source) VALUES (NEW.archive_id, NEW.archived_at, NEW.classification, NEW.legacy_desired_revision, NEW.legacy_synced_revision, NEW.legacy_user_id, NEW.local_snapshot_id, NEW.playlist_urn, NEW.prior_classification, NEW.queue_created_at, NEW.queue_failed_at, NEW.queue_generation, NEW.queue_id, NEW.queue_last_error, NEW.queue_next_run_at, NEW.queue_payload, NEW.queue_retry_count, NEW.remote_attempted_generation, NEW.remote_completed_generation, NEW.remote_result, NEW.resolved_at, NEW.source);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_legacy_membership_intents' AND p.pk = jsonb_build_object('archive_id', NEW.archive_id)) THEN
                DELETE FROM public.playlist_legacy_membership_intents t WHERE t.archive_id = OLD.archive_id;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_legacy_membership_intents' AND p.pk = jsonb_build_object('archive_id', NEW.archive_id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        k0 := NEW.queue_id IS DISTINCT FROM OLD.queue_id;
        IF k0 THEN
            FOR r IN SELECT t.archive_id FROM public.playlist_legacy_membership_intents t WHERE t.queue_id = NEW.queue_id AND t.archive_id IS DISTINCT FROM NEW.archive_id LOOP
                PERFORM public.scd_purge_playlist_legacy_membership_intents(jsonb_build_object('archive_id', r.archive_id));
            END LOOP;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.playlist_legacy_membership_intents t WHERE t.archive_id = NEW.archive_id FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_legacy_membership_intents' AND p.pk = jsonb_build_object('archive_id', NEW.archive_id)) THEN
            RETURN NULL;
        END IF;
        FOR r IN SELECT t.archive_id FROM public.playlist_legacy_membership_intents t WHERE t.queue_id = NEW.queue_id AND t.archive_id IS DISTINCT FROM NEW.archive_id LOOP
            PERFORM public.scd_purge_playlist_legacy_membership_intents(jsonb_build_object('archive_id', r.archive_id));
        END LOOP;
        IF held THEN
            UPDATE public.playlist_legacy_membership_intents t SET archived_at = NEW.archived_at, classification = NEW.classification, legacy_desired_revision = NEW.legacy_desired_revision, legacy_synced_revision = NEW.legacy_synced_revision, legacy_user_id = NEW.legacy_user_id, local_snapshot_id = NEW.local_snapshot_id, playlist_urn = NEW.playlist_urn, prior_classification = NEW.prior_classification, queue_created_at = NEW.queue_created_at, queue_failed_at = NEW.queue_failed_at, queue_generation = NEW.queue_generation, queue_id = NEW.queue_id, queue_last_error = NEW.queue_last_error, queue_next_run_at = NEW.queue_next_run_at, queue_payload = NEW.queue_payload, queue_retry_count = NEW.queue_retry_count, remote_attempted_generation = NEW.remote_attempted_generation, remote_completed_generation = NEW.remote_completed_generation, remote_result = NEW.remote_result, resolved_at = NEW.resolved_at, source = NEW.source WHERE t.archive_id = NEW.archive_id;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.playlist_legacy_membership_intents (archive_id, archived_at, classification, legacy_desired_revision, legacy_synced_revision, legacy_user_id, local_snapshot_id, playlist_urn, prior_classification, queue_created_at, queue_failed_at, queue_generation, queue_id, queue_last_error, queue_next_run_at, queue_payload, queue_retry_count, remote_attempted_generation, remote_completed_generation, remote_result, resolved_at, source)
        VALUES (NEW.archive_id, NEW.archived_at, NEW.classification, NEW.legacy_desired_revision, NEW.legacy_synced_revision, NEW.legacy_user_id, NEW.local_snapshot_id, NEW.playlist_urn, NEW.prior_classification, NEW.queue_created_at, NEW.queue_failed_at, NEW.queue_generation, NEW.queue_id, NEW.queue_last_error, NEW.queue_next_run_at, NEW.queue_payload, NEW.queue_retry_count, NEW.remote_attempted_generation, NEW.remote_completed_generation, NEW.remote_result, NEW.resolved_at, NEW.source)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.playlist_legacy_membership_intents still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.playlist_legacy_membership_intents FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_playlist_legacy_membership_intents();
ALTER TABLE public.playlist_legacy_membership_intents ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.playlist_legacy_membership_intents FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.playlist_legacy_membership_intents ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlist_membership_operations FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlist_membership_operations', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_playlist_membership_operations()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
    k0 boolean;
    k1 boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_membership_operations' AND p.pk = jsonb_build_object('operation_id', OLD.operation_id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.operation_id IS DISTINCT FROM OLD.operation_id THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_membership_operations' AND p.pk = jsonb_build_object('operation_id', OLD.operation_id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_membership_operations' AND p.pk = jsonb_build_object('operation_id', NEW.operation_id)) THEN
                    INSERT INTO public.playlist_membership_operations (accepted_projection_revision, actor_sc_user_id, base_baseline_generation, base_observation_id, boundary, conflict_code, created_at, expected_projection_revision, idempotency_key, kind, left_anchor_track_id, operation_id, ordered_track_ids, outcome, playlist_urn, request_fingerprint, resolved_at, right_anchor_track_id, sequence, track_id) VALUES (NEW.accepted_projection_revision, NEW.actor_sc_user_id, NEW.base_baseline_generation, NEW.base_observation_id, NEW.boundary, NEW.conflict_code, NEW.created_at, NEW.expected_projection_revision, NEW.idempotency_key, NEW.kind, NEW.left_anchor_track_id, NEW.operation_id, NEW.ordered_track_ids, NEW.outcome, NEW.playlist_urn, NEW.request_fingerprint, NEW.resolved_at, NEW.right_anchor_track_id, NEW.sequence, NEW.track_id);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_membership_operations' AND p.pk = jsonb_build_object('operation_id', NEW.operation_id)) THEN
                DELETE FROM public.playlist_membership_operations t WHERE t.operation_id = OLD.operation_id;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_membership_operations' AND p.pk = jsonb_build_object('operation_id', NEW.operation_id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        k0 := (NEW.playlist_urn, NEW.idempotency_key) IS DISTINCT FROM (OLD.playlist_urn, OLD.idempotency_key);
        k1 := (NEW.playlist_urn, NEW.sequence) IS DISTINCT FROM (OLD.playlist_urn, OLD.sequence);
        IF k0 THEN
            FOR r IN SELECT t.operation_id FROM public.playlist_membership_operations t WHERE (t.playlist_urn, t.idempotency_key) = (NEW.playlist_urn, NEW.idempotency_key) AND t.operation_id IS DISTINCT FROM NEW.operation_id LOOP
                PERFORM public.scd_purge_playlist_membership_operations(jsonb_build_object('operation_id', r.operation_id));
            END LOOP;
        END IF;
        IF k1 THEN
            FOR r IN SELECT t.operation_id FROM public.playlist_membership_operations t WHERE (t.playlist_urn, t.sequence) = (NEW.playlist_urn, NEW.sequence) AND t.operation_id IS DISTINCT FROM NEW.operation_id LOOP
                PERFORM public.scd_purge_playlist_membership_operations(jsonb_build_object('operation_id', r.operation_id));
            END LOOP;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.playlist_membership_operations t WHERE t.operation_id = NEW.operation_id FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_membership_operations' AND p.pk = jsonb_build_object('operation_id', NEW.operation_id)) THEN
            RETURN NULL;
        END IF;
        FOR r IN SELECT t.operation_id FROM public.playlist_membership_operations t WHERE (t.playlist_urn, t.idempotency_key) = (NEW.playlist_urn, NEW.idempotency_key) AND t.operation_id IS DISTINCT FROM NEW.operation_id LOOP
            PERFORM public.scd_purge_playlist_membership_operations(jsonb_build_object('operation_id', r.operation_id));
        END LOOP;
        FOR r IN SELECT t.operation_id FROM public.playlist_membership_operations t WHERE (t.playlist_urn, t.sequence) = (NEW.playlist_urn, NEW.sequence) AND t.operation_id IS DISTINCT FROM NEW.operation_id LOOP
            PERFORM public.scd_purge_playlist_membership_operations(jsonb_build_object('operation_id', r.operation_id));
        END LOOP;
        IF held THEN
            UPDATE public.playlist_membership_operations t SET accepted_projection_revision = NEW.accepted_projection_revision, actor_sc_user_id = NEW.actor_sc_user_id, base_baseline_generation = NEW.base_baseline_generation, base_observation_id = NEW.base_observation_id, boundary = NEW.boundary, conflict_code = NEW.conflict_code, created_at = NEW.created_at, expected_projection_revision = NEW.expected_projection_revision, idempotency_key = NEW.idempotency_key, kind = NEW.kind, left_anchor_track_id = NEW.left_anchor_track_id, ordered_track_ids = NEW.ordered_track_ids, outcome = NEW.outcome, playlist_urn = NEW.playlist_urn, request_fingerprint = NEW.request_fingerprint, resolved_at = NEW.resolved_at, right_anchor_track_id = NEW.right_anchor_track_id, sequence = NEW.sequence, track_id = NEW.track_id WHERE t.operation_id = NEW.operation_id;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.playlist_membership_operations (accepted_projection_revision, actor_sc_user_id, base_baseline_generation, base_observation_id, boundary, conflict_code, created_at, expected_projection_revision, idempotency_key, kind, left_anchor_track_id, operation_id, ordered_track_ids, outcome, playlist_urn, request_fingerprint, resolved_at, right_anchor_track_id, sequence, track_id)
        VALUES (NEW.accepted_projection_revision, NEW.actor_sc_user_id, NEW.base_baseline_generation, NEW.base_observation_id, NEW.boundary, NEW.conflict_code, NEW.created_at, NEW.expected_projection_revision, NEW.idempotency_key, NEW.kind, NEW.left_anchor_track_id, NEW.operation_id, NEW.ordered_track_ids, NEW.outcome, NEW.playlist_urn, NEW.request_fingerprint, NEW.resolved_at, NEW.right_anchor_track_id, NEW.sequence, NEW.track_id)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.playlist_membership_operations still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.playlist_membership_operations FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_playlist_membership_operations();
ALTER TABLE public.playlist_membership_operations ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.playlist_membership_operations FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.playlist_membership_operations ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlist_membership_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlist_membership_state', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_playlist_membership_state()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_membership_state' AND p.pk = jsonb_build_object('playlist_urn', OLD.playlist_urn)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.playlist_urn IS DISTINCT FROM OLD.playlist_urn THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_membership_state' AND p.pk = jsonb_build_object('playlist_urn', OLD.playlist_urn)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_membership_state' AND p.pk = jsonb_build_object('playlist_urn', NEW.playlist_urn)) THEN
                    INSERT INTO public.playlist_membership_state (baseline_generation, baseline_observation_id, candidate_fingerprint, committed_operation_sequence, conflict_code, last_error, last_operation_sequence, latest_observation_id, next_reconcile_at, owner_sc_user_id, playlist_urn, projection_revision, projection_track_count, reconcile_failure_streak, reconcile_generation, remote_applied_at, remote_apply_fingerprint, remote_apply_generation, sync_status, updated_at) VALUES (NEW.baseline_generation, NEW.baseline_observation_id, NEW.candidate_fingerprint, NEW.committed_operation_sequence, NEW.conflict_code, NEW.last_error, NEW.last_operation_sequence, NEW.latest_observation_id, NEW.next_reconcile_at, NEW.owner_sc_user_id, NEW.playlist_urn, NEW.projection_revision, NEW.projection_track_count, NEW.reconcile_failure_streak, NEW.reconcile_generation, NEW.remote_applied_at, NEW.remote_apply_fingerprint, NEW.remote_apply_generation, NEW.sync_status, NEW.updated_at);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_membership_state' AND p.pk = jsonb_build_object('playlist_urn', NEW.playlist_urn)) THEN
                DELETE FROM public.playlist_membership_state t WHERE t.playlist_urn = OLD.playlist_urn;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_membership_state' AND p.pk = jsonb_build_object('playlist_urn', NEW.playlist_urn)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.playlist_membership_state t WHERE t.playlist_urn = NEW.playlist_urn FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_membership_state' AND p.pk = jsonb_build_object('playlist_urn', NEW.playlist_urn)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.playlist_membership_state t SET baseline_generation = NEW.baseline_generation, baseline_observation_id = NEW.baseline_observation_id, candidate_fingerprint = NEW.candidate_fingerprint, committed_operation_sequence = NEW.committed_operation_sequence, conflict_code = NEW.conflict_code, last_error = NEW.last_error, last_operation_sequence = NEW.last_operation_sequence, latest_observation_id = NEW.latest_observation_id, next_reconcile_at = NEW.next_reconcile_at, owner_sc_user_id = NEW.owner_sc_user_id, projection_revision = NEW.projection_revision, projection_track_count = NEW.projection_track_count, reconcile_failure_streak = NEW.reconcile_failure_streak, reconcile_generation = NEW.reconcile_generation, remote_applied_at = NEW.remote_applied_at, remote_apply_fingerprint = NEW.remote_apply_fingerprint, remote_apply_generation = NEW.remote_apply_generation, sync_status = NEW.sync_status, updated_at = NEW.updated_at WHERE t.playlist_urn = NEW.playlist_urn;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.playlist_membership_state (baseline_generation, baseline_observation_id, candidate_fingerprint, committed_operation_sequence, conflict_code, last_error, last_operation_sequence, latest_observation_id, next_reconcile_at, owner_sc_user_id, playlist_urn, projection_revision, projection_track_count, reconcile_failure_streak, reconcile_generation, remote_applied_at, remote_apply_fingerprint, remote_apply_generation, sync_status, updated_at)
        VALUES (NEW.baseline_generation, NEW.baseline_observation_id, NEW.candidate_fingerprint, NEW.committed_operation_sequence, NEW.conflict_code, NEW.last_error, NEW.last_operation_sequence, NEW.latest_observation_id, NEW.next_reconcile_at, NEW.owner_sc_user_id, NEW.playlist_urn, NEW.projection_revision, NEW.projection_track_count, NEW.reconcile_failure_streak, NEW.reconcile_generation, NEW.remote_applied_at, NEW.remote_apply_fingerprint, NEW.remote_apply_generation, NEW.sync_status, NEW.updated_at)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.playlist_membership_state still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.playlist_membership_state FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_playlist_membership_state();
ALTER TABLE public.playlist_membership_state ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.playlist_membership_state FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.playlist_membership_state ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlist_reconcile_runs FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlist_reconcile_runs', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlist_remote_observations FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlist_remote_observations', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_playlist_remote_observations()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
    k0 boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_remote_observations' AND p.pk = jsonb_build_object('id', OLD.id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.id IS DISTINCT FROM OLD.id THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_remote_observations' AND p.pk = jsonb_build_object('id', OLD.id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_remote_observations' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                    INSERT INTO public.playlist_remote_observations (all_items_identified, authority, declared_track_count, error_kind, id, observed_at, observed_track_count, outcome, pagination_complete, playlist_urn, retry_at, sc_last_modified, snapshot_id) VALUES (NEW.all_items_identified, NEW.authority, NEW.declared_track_count, NEW.error_kind, NEW.id, NEW.observed_at, NEW.observed_track_count, NEW.outcome, NEW.pagination_complete, NEW.playlist_urn, NEW.retry_at, NEW.sc_last_modified, NEW.snapshot_id);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_remote_observations' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                DELETE FROM public.playlist_remote_observations t WHERE t.id = OLD.id;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_remote_observations' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        k0 := (NEW.playlist_urn, NEW.id) IS DISTINCT FROM (OLD.playlist_urn, OLD.id);
        IF k0 THEN
            FOR r IN SELECT t.id FROM public.playlist_remote_observations t WHERE (t.playlist_urn, t.id) = (NEW.playlist_urn, NEW.id) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_playlist_remote_observations(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.playlist_remote_observations t WHERE t.id = NEW.id FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_remote_observations' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
            RETURN NULL;
        END IF;
        FOR r IN SELECT t.id FROM public.playlist_remote_observations t WHERE (t.playlist_urn, t.id) = (NEW.playlist_urn, NEW.id) AND t.id IS DISTINCT FROM NEW.id LOOP
            PERFORM public.scd_purge_playlist_remote_observations(jsonb_build_object('id', r.id));
        END LOOP;
        IF held THEN
            UPDATE public.playlist_remote_observations t SET all_items_identified = NEW.all_items_identified, authority = NEW.authority, declared_track_count = NEW.declared_track_count, error_kind = NEW.error_kind, observed_at = NEW.observed_at, observed_track_count = NEW.observed_track_count, outcome = NEW.outcome, pagination_complete = NEW.pagination_complete, playlist_urn = NEW.playlist_urn, retry_at = NEW.retry_at, sc_last_modified = NEW.sc_last_modified, snapshot_id = NEW.snapshot_id WHERE t.id = NEW.id;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.playlist_remote_observations (all_items_identified, authority, declared_track_count, error_kind, id, observed_at, observed_track_count, outcome, pagination_complete, playlist_urn, retry_at, sc_last_modified, snapshot_id)
        VALUES (NEW.all_items_identified, NEW.authority, NEW.declared_track_count, NEW.error_kind, NEW.id, NEW.observed_at, NEW.observed_track_count, NEW.outcome, NEW.pagination_complete, NEW.playlist_urn, NEW.retry_at, NEW.sc_last_modified, NEW.snapshot_id)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.playlist_remote_observations still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.playlist_remote_observations FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_playlist_remote_observations();
ALTER TABLE public.playlist_remote_observations ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.playlist_remote_observations FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.playlist_remote_observations ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlist_remote_snapshot_tracks FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlist_remote_snapshot_tracks', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlist_remote_snapshots FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlist_remote_snapshots', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_playlist_remote_snapshots()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
    k0 boolean;
    k1 boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_remote_snapshots' AND p.pk = jsonb_build_object('id', OLD.id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.id IS DISTINCT FROM OLD.id THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_remote_snapshots' AND p.pk = jsonb_build_object('id', OLD.id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_remote_snapshots' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                    INSERT INTO public.playlist_remote_snapshots (content_fingerprint, created_at, id, playlist_urn, track_count) VALUES (NEW.content_fingerprint, NEW.created_at, NEW.id, NEW.playlist_urn, NEW.track_count);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_remote_snapshots' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                DELETE FROM public.playlist_remote_snapshots t WHERE t.id = OLD.id;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_remote_snapshots' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        k0 := (NEW.playlist_urn, NEW.content_fingerprint) IS DISTINCT FROM (OLD.playlist_urn, OLD.content_fingerprint);
        k1 := (NEW.playlist_urn, NEW.id) IS DISTINCT FROM (OLD.playlist_urn, OLD.id);
        IF k0 THEN
            FOR r IN SELECT t.id FROM public.playlist_remote_snapshots t WHERE (t.playlist_urn, t.content_fingerprint) = (NEW.playlist_urn, NEW.content_fingerprint) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_playlist_remote_snapshots(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        IF k1 THEN
            FOR r IN SELECT t.id FROM public.playlist_remote_snapshots t WHERE (t.playlist_urn, t.id) = (NEW.playlist_urn, NEW.id) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_playlist_remote_snapshots(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.playlist_remote_snapshots t WHERE t.id = NEW.id FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_remote_snapshots' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
            RETURN NULL;
        END IF;
        FOR r IN SELECT t.id FROM public.playlist_remote_snapshots t WHERE (t.playlist_urn, t.content_fingerprint) = (NEW.playlist_urn, NEW.content_fingerprint) AND t.id IS DISTINCT FROM NEW.id LOOP
            PERFORM public.scd_purge_playlist_remote_snapshots(jsonb_build_object('id', r.id));
        END LOOP;
        FOR r IN SELECT t.id FROM public.playlist_remote_snapshots t WHERE (t.playlist_urn, t.id) = (NEW.playlist_urn, NEW.id) AND t.id IS DISTINCT FROM NEW.id LOOP
            PERFORM public.scd_purge_playlist_remote_snapshots(jsonb_build_object('id', r.id));
        END LOOP;
        IF held THEN
            UPDATE public.playlist_remote_snapshots t SET content_fingerprint = NEW.content_fingerprint, created_at = NEW.created_at, playlist_urn = NEW.playlist_urn, track_count = NEW.track_count WHERE t.id = NEW.id;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.playlist_remote_snapshots (content_fingerprint, created_at, id, playlist_urn, track_count)
        VALUES (NEW.content_fingerprint, NEW.created_at, NEW.id, NEW.playlist_urn, NEW.track_count)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.playlist_remote_snapshots still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.playlist_remote_snapshots FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_playlist_remote_snapshots();
ALTER TABLE public.playlist_remote_snapshots ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.playlist_remote_snapshots FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.playlist_remote_snapshots ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlist_sweep_cursor FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlist_sweep_cursor', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlist_track_projection FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlist_track_projection', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_playlist_track_projection()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
    k0 boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_track_projection' AND p.pk = jsonb_build_object('playlist_urn', OLD.playlist_urn, 'position', OLD."position")) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.playlist_urn, NEW."position") IS DISTINCT FROM (OLD.playlist_urn, OLD."position") THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_track_projection' AND p.pk = jsonb_build_object('playlist_urn', OLD.playlist_urn, 'position', OLD."position")) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_track_projection' AND p.pk = jsonb_build_object('playlist_urn', NEW.playlist_urn, 'position', NEW."position")) THEN
                    INSERT INTO public.playlist_track_projection (playlist_urn, "position", sc_track_id) VALUES (NEW.playlist_urn, NEW."position", NEW.sc_track_id);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_track_projection' AND p.pk = jsonb_build_object('playlist_urn', NEW.playlist_urn, 'position', NEW."position")) THEN
                DELETE FROM public.playlist_track_projection t WHERE (t.playlist_urn, t."position") = (OLD.playlist_urn, OLD."position");
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_track_projection' AND p.pk = jsonb_build_object('playlist_urn', NEW.playlist_urn, 'position', NEW."position")) THEN
                RETURN NULL;
            END IF;
        END IF;
        k0 := (NEW.playlist_urn, NEW.sc_track_id) IS DISTINCT FROM (OLD.playlist_urn, OLD.sc_track_id);
        IF k0 THEN
            FOR r IN SELECT t.playlist_urn, t."position" FROM public.playlist_track_projection t WHERE (t.playlist_urn, t.sc_track_id) = (NEW.playlist_urn, NEW.sc_track_id) AND (t.playlist_urn, t."position") IS DISTINCT FROM (NEW.playlist_urn, NEW."position") LOOP
                PERFORM public.scd_purge_playlist_track_projection(jsonb_build_object('playlist_urn', r.playlist_urn, 'position', r."position"));
            END LOOP;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.playlist_track_projection t WHERE (t.playlist_urn, t."position") = (NEW.playlist_urn, NEW."position") FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_track_projection' AND p.pk = jsonb_build_object('playlist_urn', NEW.playlist_urn, 'position', NEW."position")) THEN
            RETURN NULL;
        END IF;
        FOR r IN SELECT t.playlist_urn, t."position" FROM public.playlist_track_projection t WHERE (t.playlist_urn, t.sc_track_id) = (NEW.playlist_urn, NEW.sc_track_id) AND (t.playlist_urn, t."position") IS DISTINCT FROM (NEW.playlist_urn, NEW."position") LOOP
            PERFORM public.scd_purge_playlist_track_projection(jsonb_build_object('playlist_urn', r.playlist_urn, 'position', r."position"));
        END LOOP;
        IF held THEN
            UPDATE public.playlist_track_projection t SET sc_track_id = NEW.sc_track_id WHERE (t.playlist_urn, t."position") = (NEW.playlist_urn, NEW."position");
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.playlist_track_projection (playlist_urn, "position", sc_track_id)
        VALUES (NEW.playlist_urn, NEW."position", NEW.sc_track_id)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.playlist_track_projection still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.playlist_track_projection FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_playlist_track_projection();
ALTER TABLE public.playlist_track_projection ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.playlist_track_projection FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.playlist_track_projection ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_core_col_playlists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF NEW.last_read_at IS DISTINCT FROM OLD.last_read_at THEN
        NEW.last_read_at := OLD.last_read_at;
        PERFORM nextval('public.scd_core_col_playlists'::regclass);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_core_col BEFORE UPDATE OF last_read_at ON public.playlists FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_core_col_playlists();

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlists FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlists', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_playlists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
    k0 boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlists' AND p.pk = jsonb_build_object('urn', OLD.urn)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.urn IS DISTINCT FROM OLD.urn THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlists' AND p.pk = jsonb_build_object('urn', OLD.urn)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlists' AND p.pk = jsonb_build_object('urn', NEW.urn)) THEN
                    INSERT INTO public.playlists (artwork_url, created_at, deleted_at, description, duration_ms, genre, kind, label_name, last_read_at, likes_count_sc, owner_sc_user_id, owner_urn, owner_username, permalink_url, playlist_type, release_date, release_year, reposts_count_sc, sc_created_at, sc_desired, sc_last_modified, sc_metadata, sc_mutation_observation, sc_observation, sc_playlist_id, sc_synced_at, sc_write_confirmed, sharing, tags, title, title_normalized, track_count, updated_at, urn) VALUES (NEW.artwork_url, NEW.created_at, NEW.deleted_at, NEW.description, NEW.duration_ms, NEW.genre, NEW.kind, NEW.label_name, NEW.last_read_at, NEW.likes_count_sc, NEW.owner_sc_user_id, NEW.owner_urn, NEW.owner_username, NEW.permalink_url, NEW.playlist_type, NEW.release_date, NEW.release_year, NEW.reposts_count_sc, NEW.sc_created_at, NEW.sc_desired, NEW.sc_last_modified, NEW.sc_metadata, NEW.sc_mutation_observation, NEW.sc_observation, NEW.sc_playlist_id, NEW.sc_synced_at, NEW.sc_write_confirmed, NEW.sharing, NEW.tags, NEW.title, NEW.title_normalized, NEW.track_count, NEW.updated_at, NEW.urn);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlists' AND p.pk = jsonb_build_object('urn', NEW.urn)) THEN
                DELETE FROM public.playlists t WHERE t.urn = OLD.urn;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlists' AND p.pk = jsonb_build_object('urn', NEW.urn)) THEN
                IF NEW.last_read_at IS DISTINCT FROM OLD.last_read_at THEN
                    NEW.artwork_url := OLD.artwork_url;
                    NEW.created_at := OLD.created_at;
                    NEW.deleted_at := OLD.deleted_at;
                    NEW.description := OLD.description;
                    NEW.duration_ms := OLD.duration_ms;
                    NEW.genre := OLD.genre;
                    NEW.kind := OLD.kind;
                    NEW.label_name := OLD.label_name;
                    NEW.likes_count_sc := OLD.likes_count_sc;
                    NEW.owner_sc_user_id := OLD.owner_sc_user_id;
                    NEW.owner_urn := OLD.owner_urn;
                    NEW.owner_username := OLD.owner_username;
                    NEW.permalink_url := OLD.permalink_url;
                    NEW.playlist_type := OLD.playlist_type;
                    NEW.release_date := OLD.release_date;
                    NEW.release_year := OLD.release_year;
                    NEW.reposts_count_sc := OLD.reposts_count_sc;
                    NEW.sc_created_at := OLD.sc_created_at;
                    NEW.sc_desired := OLD.sc_desired;
                    NEW.sc_last_modified := OLD.sc_last_modified;
                    NEW.sc_metadata := OLD.sc_metadata;
                    NEW.sc_mutation_observation := OLD.sc_mutation_observation;
                    NEW.sc_observation := OLD.sc_observation;
                    NEW.sc_playlist_id := OLD.sc_playlist_id;
                    NEW.sc_synced_at := OLD.sc_synced_at;
                    NEW.sc_write_confirmed := OLD.sc_write_confirmed;
                    NEW.sharing := OLD.sharing;
                    NEW.tags := OLD.tags;
                    NEW.title := OLD.title;
                    NEW.title_normalized := OLD.title_normalized;
                    NEW.track_count := OLD.track_count;
                    NEW.updated_at := OLD.updated_at;
                    RETURN NEW;
                END IF;
                RETURN NULL;
            END IF;
        END IF;
        k0 := NEW.sc_playlist_id IS DISTINCT FROM OLD.sc_playlist_id;
        IF k0 THEN
            FOR r IN SELECT t.urn FROM public.playlists t WHERE t.sc_playlist_id = NEW.sc_playlist_id AND t.urn IS DISTINCT FROM NEW.urn LOOP
                PERFORM public.scd_purge_playlists(jsonb_build_object('urn', r.urn));
            END LOOP;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.playlists t WHERE t.urn = NEW.urn FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlists' AND p.pk = jsonb_build_object('urn', NEW.urn)) THEN
            PERFORM pg_catalog.set_config('scd.merging', 'on', true);
            UPDATE public.playlists t SET last_read_at = NEW.last_read_at WHERE t.urn = NEW.urn AND t.last_read_at IS DISTINCT FROM NEW.last_read_at;
            PERFORM pg_catalog.set_config('scd.merging', '', true);
            RETURN NULL;
        END IF;
        FOR r IN SELECT t.urn FROM public.playlists t WHERE t.sc_playlist_id = NEW.sc_playlist_id AND t.urn IS DISTINCT FROM NEW.urn LOOP
            PERFORM public.scd_purge_playlists(jsonb_build_object('urn', r.urn));
        END LOOP;
        IF held THEN
            UPDATE public.playlists t SET artwork_url = NEW.artwork_url, created_at = NEW.created_at, deleted_at = NEW.deleted_at, description = NEW.description, duration_ms = NEW.duration_ms, genre = NEW.genre, kind = NEW.kind, label_name = NEW.label_name, last_read_at = NEW.last_read_at, likes_count_sc = NEW.likes_count_sc, owner_sc_user_id = NEW.owner_sc_user_id, owner_urn = NEW.owner_urn, owner_username = NEW.owner_username, permalink_url = NEW.permalink_url, playlist_type = NEW.playlist_type, release_date = NEW.release_date, release_year = NEW.release_year, reposts_count_sc = NEW.reposts_count_sc, sc_created_at = NEW.sc_created_at, sc_desired = NEW.sc_desired, sc_last_modified = NEW.sc_last_modified, sc_metadata = NEW.sc_metadata, sc_mutation_observation = NEW.sc_mutation_observation, sc_observation = NEW.sc_observation, sc_playlist_id = NEW.sc_playlist_id, sc_synced_at = NEW.sc_synced_at, sc_write_confirmed = NEW.sc_write_confirmed, sharing = NEW.sharing, tags = NEW.tags, title = NEW.title, title_normalized = NEW.title_normalized, track_count = NEW.track_count, updated_at = NEW.updated_at WHERE t.urn = NEW.urn;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.playlists (artwork_url, created_at, deleted_at, description, duration_ms, genre, kind, label_name, last_read_at, likes_count_sc, owner_sc_user_id, owner_urn, owner_username, permalink_url, playlist_type, release_date, release_year, reposts_count_sc, sc_created_at, sc_desired, sc_last_modified, sc_metadata, sc_mutation_observation, sc_observation, sc_playlist_id, sc_synced_at, sc_write_confirmed, sharing, tags, title, title_normalized, track_count, updated_at, urn)
        VALUES (NEW.artwork_url, NEW.created_at, NEW.deleted_at, NEW.description, NEW.duration_ms, NEW.genre, NEW.kind, NEW.label_name, NEW.last_read_at, NEW.likes_count_sc, NEW.owner_sc_user_id, NEW.owner_urn, NEW.owner_username, NEW.permalink_url, NEW.playlist_type, NEW.release_date, NEW.release_year, NEW.reposts_count_sc, NEW.sc_created_at, NEW.sc_desired, NEW.sc_last_modified, NEW.sc_metadata, NEW.sc_mutation_observation, NEW.sc_observation, NEW.sc_playlist_id, NEW.sc_synced_at, NEW.sc_write_confirmed, NEW.sharing, NEW.tags, NEW.title, NEW.title_normalized, NEW.track_count, NEW.updated_at, NEW.urn)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.playlists still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.playlists FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_playlists();
ALTER TABLE public.playlists ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.playlists FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.playlists ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.recommendation_quality_models FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_recommendation_quality_models', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_max_repl_checksums()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    n bigint;
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF current_setting('scd.merging', true) = 'on' THEN
            RETURN NEW;
        END IF;
        FOR attempt IN 1..3 LOOP
            UPDATE public.repl_checksums t SET at = NEW.at, hash = NEW.hash WHERE (t.node, t.tbl, t.range) = (NEW.node, NEW.tbl, NEW.range) AND (t.at IS NULL OR t.at < NEW.at);
            IF FOUND THEN
                RETURN NULL;
            END IF;
            PERFORM 1 FROM public.repl_checksums t WHERE (t.node, t.tbl, t.range) = (NEW.node, NEW.tbl, NEW.range);
            IF FOUND THEN
                RETURN NULL;
            END IF;
            PERFORM pg_catalog.set_config('scd.merging', 'on', true);
            INSERT INTO public.repl_checksums (at, hash, node, range, tbl)
            VALUES (NEW.at, NEW.hash, NEW.node, NEW.range, NEW.tbl)
            ON CONFLICT DO NOTHING;
            GET DIAGNOSTICS n = ROW_COUNT;
            PERFORM pg_catalog.set_config('scd.merging', '', true);
            IF n > 0 THEN
                RETURN NULL;
            END IF;
        END LOOP;
        RAISE EXCEPTION 'scd: insert into public.repl_checksums still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
    END IF;
    IF OLD.at IS NOT NULL AND (NEW.at IS NULL OR NEW.at <= OLD.at) THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_max BEFORE INSERT OR UPDATE ON public.repl_checksums FOR EACH ROW WHEN (public.scd_role() = 'core' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_max_repl_checksums();
ALTER TABLE public.repl_checksums ENABLE REPLICA TRIGGER zz_max;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.repl_checksums FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.repl_checksums ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_tomb_lock(tbl text, k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    CASE tbl
        WHEN 'discover_promoted' THEN
            PERFORM 1 FROM public.discover_promoted t WHERE t.id = (k ->> 'id')::uuid FOR UPDATE;
        WHEN 'disliked_tracks' THEN
            PERFORM 1 FROM public.disliked_tracks t WHERE t.id = (k ->> 'id')::uuid FOR UPDATE;
        WHEN 'featured_items' THEN
            PERFORM 1 FROM public.featured_items t WHERE t.id = (k ->> 'id')::uuid FOR UPDATE;
        WHEN 'link_requests' THEN
            PERFORM 1 FROM public.link_requests t WHERE t.id = (k ->> 'id')::uuid FOR UPDATE;
        WHEN 'listening_history' THEN
            PERFORM 1 FROM public.listening_history t WHERE t.id = (k ->> 'id')::uuid FOR UPDATE;
        WHEN 'login_requests' THEN
            PERFORM 1 FROM public.login_requests t WHERE t.id = (k ->> 'id')::uuid FOR UPDATE;
        WHEN 'sc_egress_health' THEN
            PERFORM 1 FROM public.sc_egress_health t WHERE t.channel = (k ->> 'channel')::text FOR UPDATE;
        WHEN 'sessions' THEN
            PERFORM 1 FROM public.sessions t WHERE t.id = (k ->> 'id')::uuid FOR UPDATE;
        WHEN 'subscriptions' THEN
            PERFORM 1 FROM public.subscriptions t WHERE t.user_urn = (k ->> 'user_urn')::text FOR UPDATE;
        WHEN 'track_artist_blocks' THEN
            PERFORM 1 FROM public.track_artist_blocks t WHERE t.track_id = (k ->> 'track_id')::uuid AND t.artist_id = (k ->> 'artist_id')::uuid FOR UPDATE;
        WHEN 'user_blocked_artists' THEN
            PERFORM 1 FROM public.user_blocked_artists t WHERE t.sc_user_id = (k ->> 'sc_user_id')::text AND t.kind = (k ->> 'kind')::text AND t.target_id = (k ->> 'target_id')::text FOR UPDATE;
        WHEN 'user_like_ranks' THEN
            PERFORM 1 FROM public.user_like_ranks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text FOR UPDATE;
        WHEN 'user_superlike_ranks' THEN
            PERFORM 1 FROM public.user_superlike_ranks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text FOR UPDATE;
        WHEN 'user_superlikes' THEN
            PERFORM 1 FROM public.user_superlikes t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text FOR UPDATE;
        ELSE
            NULL;
    END CASE;
END
$f$;
CREATE OR REPLACE FUNCTION public.scd_erase(tbl text, k jsonb, upto bigint)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    CASE tbl
        WHEN 'discover_promoted' THEN
            PERFORM public.scd_erase_discover_promoted(k, upto);
        WHEN 'disliked_tracks' THEN
            PERFORM public.scd_erase_disliked_tracks(k, upto);
        WHEN 'featured_items' THEN
            PERFORM public.scd_erase_featured_items(k, upto);
        WHEN 'link_requests' THEN
            PERFORM public.scd_erase_link_requests(k, upto);
        WHEN 'listening_history' THEN
            PERFORM public.scd_erase_listening_history(k, upto);
        WHEN 'login_requests' THEN
            PERFORM public.scd_erase_login_requests(k, upto);
        WHEN 'sc_egress_health' THEN
            PERFORM public.scd_erase_sc_egress_health(k, upto);
        WHEN 'sessions' THEN
            PERFORM public.scd_erase_sessions(k, upto);
        WHEN 'subscriptions' THEN
            PERFORM public.scd_erase_subscriptions(k, upto);
        WHEN 'track_artist_blocks' THEN
            PERFORM public.scd_erase_track_artist_blocks(k, upto);
        WHEN 'user_blocked_artists' THEN
            PERFORM public.scd_erase_user_blocked_artists(k, upto);
        WHEN 'user_like_ranks' THEN
            PERFORM public.scd_erase_user_like_ranks(k, upto);
        WHEN 'user_superlike_ranks' THEN
            PERFORM public.scd_erase_user_superlike_ranks(k, upto);
        WHEN 'user_superlikes' THEN
            PERFORM public.scd_erase_user_superlikes(k, upto);
        ELSE
            NULL;
    END CASE;
END
$f$;
CREATE OR REPLACE FUNCTION public.scd_erase_repl_tombstones()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    PERFORM public.scd_tomb_lock(NEW.tbl, NEW.pk);
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    INSERT INTO public.repl_tombstones (tbl, pk, v, at) VALUES (NEW.tbl, NEW.pk, NEW.v, NEW.at) ON CONFLICT DO NOTHING;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
    PERFORM public.scd_erase(NEW.tbl, NEW.pk, NEW.v);
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_erase BEFORE INSERT OR UPDATE ON public.repl_tombstones FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_erase_repl_tombstones();
ALTER TABLE public.repl_tombstones ENABLE REPLICA TRIGGER zz_erase;

CREATE OR REPLACE FUNCTION public.scd_ver_repl_tombstones()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'UPDATE' THEN
        RAISE EXCEPTION 'scd: public.repl_tombstones rows are immutable, a delete adds a row' USING ERRCODE = 'feature_not_supported',
            HINT = 'to send tombstones again run public.scd_resend_all() and drain with public.scd_drain_resend()';
    END IF;
    NEW.v := public.scd_hlc(greatest(NEW.v, (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = NEW.tbl AND x.pk = NEW.pk)));
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.repl_tombstones FOR EACH ROW EXECUTE FUNCTION public.scd_ver_repl_tombstones();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.repl_tombstones FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.repl_tombstones ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_merge_sc_egress_health()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.open_until := OLD.open_until;
            NEW.opened_at := OLD.opened_at;
            NEW.opened_by := OLD.opened_by;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.sc_egress_health t WHERE t.channel = NEW.channel;
        IF FOUND THEN
            IF NEW.row_v > o.row_v THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.sc_egress_health t SET
                    open_until = CASE WHEN NEW.row_v > o.row_v THEN NEW.open_until ELSE o.open_until END,
                    opened_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.opened_at ELSE o.opened_at END,
                    opened_by = CASE WHEN NEW.row_v > o.row_v THEN NEW.opened_by ELSE o.opened_by END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE t.channel = o.channel;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'sc_egress_health' AND x.pk = jsonb_build_object('channel', NEW.channel));
        IF tv IS NOT NULL AND NEW.row_v <= tv THEN
            INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'sc_egress_health', 'pk', jsonb_build_object('channel', NEW.channel), 'v', tv)) ON CONFLICT DO NOTHING;
            PERFORM nextval('public.scd_resend_sc_egress_health'::regclass);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.sc_egress_health (channel, open_until, opened_at, opened_by, row_v)
        VALUES (NEW.channel, NEW.open_until, NEW.opened_at, NEW.opened_by, NEW.row_v)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.sc_egress_health still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.sc_egress_health FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_sc_egress_health();
ALTER TABLE public.sc_egress_health ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_sc_egress_health()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('sc_egress_health', jsonb_build_object('channel', OLD.channel), OLD.row_v, now());
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb BEFORE DELETE ON public.sc_egress_health FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_sc_egress_health();
CREATE OR REPLACE FUNCTION public.scd_erase_sc_egress_health(k jsonb, upto bigint)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    DELETE FROM public.sc_egress_health t WHERE t.channel = (k ->> 'channel')::text AND t.row_v <= upto;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
    IF EXISTS (SELECT 1 FROM public.sc_egress_health t WHERE t.channel = (k ->> 'channel')::text) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('sc_egress_health', k) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_sc_egress_health'::regclass);
    END IF;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_tomb_check_sc_egress_health()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'sc_egress_health' AND x.pk = jsonb_build_object('channel', NEW.channel));
    IF tv IS NOT NULL AND NEW.row_v <= tv THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'sc_egress_health', 'pk', jsonb_build_object('channel', NEW.channel), 'v', tv)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_sc_egress_health'::regclass);
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.sc_egress_health FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_sc_egress_health();
ALTER TABLE public.sc_egress_health ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_tomb_guard_sc_egress_health()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN OLD;
    END IF;
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'sc_egress_health' AND x.pk = jsonb_build_object('channel', OLD.channel));
    IF tv IS NULL OR OLD.row_v <= tv THEN
        RETURN OLD;
    END IF;
    INSERT INTO public.repl_resend (tbl, pk) VALUES ('sc_egress_health', jsonb_build_object('channel', OLD.channel)) ON CONFLICT DO NOTHING;
    PERFORM nextval('public.scd_resend_sc_egress_health'::regclass);
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_guard BEFORE DELETE ON public.sc_egress_health FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_guard_sc_egress_health();
ALTER TABLE public.sc_egress_health ENABLE REPLICA TRIGGER zz_tomb_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_sc_egress_health()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(greatest(NEW.row_v, (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'sc_egress_health' AND x.pk = jsonb_build_object('channel', NEW.channel))));
        RETURN NEW;
    END IF;
    IF (NEW.open_until, NEW.opened_at, NEW.opened_by) IS DISTINCT FROM (OLD.open_until, OLD.opened_at, OLD.opened_by) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.sc_egress_health FOR EACH ROW EXECUTE FUNCTION public.scd_ver_sc_egress_health();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.sc_egress_health FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.sc_egress_health ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_max_sc_track_counters()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    n bigint;
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF current_setting('scd.merging', true) = 'on' THEN
            RETURN NEW;
        END IF;
        FOR attempt IN 1..3 LOOP
            UPDATE public.sc_track_counters t SET comment_count = NEW.comment_count, fetched_at = NEW.fetched_at, likes_count = NEW.likes_count, play_count = NEW.play_count, reposts_count = NEW.reposts_count WHERE t.sc_track_id = NEW.sc_track_id AND (t.fetched_at IS NULL OR t.fetched_at < NEW.fetched_at);
            IF FOUND THEN
                RETURN NULL;
            END IF;
            PERFORM 1 FROM public.sc_track_counters t WHERE t.sc_track_id = NEW.sc_track_id;
            IF FOUND THEN
                RETURN NULL;
            END IF;
            PERFORM pg_catalog.set_config('scd.merging', 'on', true);
            INSERT INTO public.sc_track_counters (comment_count, fetched_at, likes_count, play_count, reposts_count, sc_track_id)
            VALUES (NEW.comment_count, NEW.fetched_at, NEW.likes_count, NEW.play_count, NEW.reposts_count, NEW.sc_track_id)
            ON CONFLICT DO NOTHING;
            GET DIAGNOSTICS n = ROW_COUNT;
            PERFORM pg_catalog.set_config('scd.merging', '', true);
            IF n > 0 THEN
                RETURN NULL;
            END IF;
        END LOOP;
        RAISE EXCEPTION 'scd: insert into public.sc_track_counters still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
    END IF;
    IF OLD.fetched_at IS NOT NULL AND (NEW.fetched_at IS NULL OR NEW.fetched_at <= OLD.fetched_at) THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_max BEFORE INSERT OR UPDATE ON public.sc_track_counters FOR EACH ROW WHEN (public.scd_role() = 'core' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_max_sc_track_counters();
ALTER TABLE public.sc_track_counters ENABLE REPLICA TRIGGER zz_max;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.sc_track_counters FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.sc_track_counters ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.search_terms FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_search_terms', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_search_terms()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'search_terms' AND p.pk = jsonb_build_object('word', OLD.word)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.word IS DISTINCT FROM OLD.word THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'search_terms' AND p.pk = jsonb_build_object('word', OLD.word)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'search_terms' AND p.pk = jsonb_build_object('word', NEW.word)) THEN
                    INSERT INTO public.search_terms (latin, ndoc, word) VALUES (NEW.latin, NEW.ndoc, NEW.word);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'search_terms' AND p.pk = jsonb_build_object('word', NEW.word)) THEN
                DELETE FROM public.search_terms t WHERE t.word = OLD.word;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'search_terms' AND p.pk = jsonb_build_object('word', NEW.word)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.search_terms t WHERE t.word = NEW.word FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'search_terms' AND p.pk = jsonb_build_object('word', NEW.word)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.search_terms t SET latin = NEW.latin, ndoc = NEW.ndoc WHERE t.word = NEW.word;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.search_terms (latin, ndoc, word)
        VALUES (NEW.latin, NEW.ndoc, NEW.word)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.search_terms still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.search_terms FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_search_terms();
ALTER TABLE public.search_terms ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.search_terms FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.search_terms ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_merge_sessions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.created_at := OLD.created_at;
            NEW.soundcloud_connection_id := OLD.soundcloud_connection_id;
            NEW.updated_at := OLD.updated_at;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.sessions t WHERE t.id = NEW.id;
        IF FOUND THEN
            IF NEW.row_v > o.row_v OR (((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) AND NEW.created_at < o.created_at) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.sessions t SET
                    created_at = CASE WHEN ((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.row_v > o.row_v THEN NEW.created_at ELSE o.created_at END,
                    soundcloud_connection_id = CASE WHEN NEW.row_v > o.row_v THEN NEW.soundcloud_connection_id ELSE o.soundcloud_connection_id END,
                    updated_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.updated_at ELSE o.updated_at END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE t.id = o.id;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'sessions' AND x.pk = jsonb_build_object('id', NEW.id));
        IF tv IS NOT NULL THEN
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.sessions (created_at, id, row_v, soundcloud_connection_id, updated_at)
        VALUES (NEW.created_at, NEW.id, NEW.row_v, NEW.soundcloud_connection_id, NEW.updated_at)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.sessions still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.sessions FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_sessions();
ALTER TABLE public.sessions ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_orphan_sessions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF NEW.soundcloud_connection_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM public.soundcloud_connections p WHERE p.id = NEW.soundcloud_connection_id) THEN
        PERFORM nextval('public.scd_orphan_sessions'::regclass);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_orphan BEFORE INSERT OR UPDATE OF soundcloud_connection_id ON public.sessions FOR EACH ROW WHEN (public.scd_role() = 'core' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_orphan_sessions();
ALTER TABLE public.sessions ENABLE REPLICA TRIGGER zz_orphan;

CREATE OR REPLACE FUNCTION public.scd_tomb_sessions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('sessions', jsonb_build_object('id', OLD.id), OLD.row_v, now());
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb BEFORE DELETE ON public.sessions FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_sessions();
CREATE OR REPLACE FUNCTION public.scd_erase_sessions(k jsonb, upto bigint)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    DELETE FROM public.sessions t WHERE t.id = (k ->> 'id')::uuid;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_tomb_check_sessions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'sessions' AND x.pk = jsonb_build_object('id', NEW.id));
    IF tv IS NOT NULL THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.sessions FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_sessions();
ALTER TABLE public.sessions ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_ver_sessions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.created_at, NEW.soundcloud_connection_id, NEW.updated_at) IS DISTINCT FROM (OLD.created_at, OLD.soundcloud_connection_id, OLD.updated_at) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.sessions FOR EACH ROW EXECUTE FUNCTION public.scd_ver_sessions();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.sessions FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.sessions ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
CREATE OR REPLACE FUNCTION public.scd_row_hash_soundcloud_connections(k jsonb)
RETURNS text
LANGUAGE sql STABLE
SET timezone TO 'UTC'
SET extra_float_digits TO 1
SET datestyle TO 'ISO, YMD'
SET intervalstyle TO 'postgres'
SET bytea_output TO 'hex'
AS $f$
SELECT md5(ROW(t.access_token, t.created_at, t.expires_at, t.first_refresh_rejection_at, t.id, t.last_refresh_attempt_at, t.last_refresh_error, t.last_refresh_error_kind, t.last_refresh_success_at, t.login_v, t.oauth_app_id, t.refresh_failure_count, t.refresh_generation, t.refresh_lease_expires_at, t.refresh_lease_id, t.refresh_rejection_count, t.refresh_token, t.retry_at, t.scope, t.soundcloud_user_id, t.token_v, t.updated_at, t.username)::text) FROM public.soundcloud_connections t WHERE t.id = (k ->> 'id')::uuid
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

DROP FUNCTION IF EXISTS public.scd_purge_intent_soundcloud_connections(jsonb);
CREATE OR REPLACE FUNCTION public.scd_purge_intent_soundcloud_connections(k jsonb, seen bigint)
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
    IF NOT FOUND OR v IS DISTINCT FROM seen THEN
        RETURN false;
    END IF;
    INSERT INTO public.repl_purges (tbl, pk, purged_v) VALUES ('soundcloud_connections', key, v);
    DELETE FROM public.soundcloud_connections t WHERE t.id = (k ->> 'id')::uuid;
    RETURN true;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_fk_soundcloud_connections()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    UPDATE public.sessions c SET soundcloud_connection_id = NULL WHERE c.soundcloud_connection_id = OLD.id;
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_fk BEFORE DELETE ON public.soundcloud_connections FOR EACH ROW WHEN (public.scd_role() = 'core' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_fk_soundcloud_connections();
ALTER TABLE public.soundcloud_connections ENABLE REPLICA TRIGGER zz_fk;

CREATE OR REPLACE TRIGGER zz_guard_result BEFORE UPDATE OF first_refresh_rejection_at, last_refresh_attempt_at, last_refresh_error, last_refresh_error_kind, last_refresh_success_at, login_v, refresh_failure_count, refresh_generation, refresh_lease_expires_at, refresh_lease_id, refresh_rejection_count, retry_at, updated_at ON public.soundcloud_connections FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_result_soundcloud_connections', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_merge_soundcloud_connections()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
    pv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF NOT core THEN
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'soundcloud_connections' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
            RETURN NULL;
        END IF;
    END IF;
    pv := (SELECT max(x.purged_v) FROM public.repl_purges x WHERE x.tbl = 'soundcloud_connections' AND x.pk = jsonb_build_object('id', NEW.id));
    IF pv IS NOT NULL AND NEW.token_v <= pv THEN
        PERFORM nextval('public.scd_purge_block_soundcloud_connections'::regclass);
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.token_v < OLD.token_v OR (core AND NEW.token_v = OLD.token_v) THEN
            NEW.soundcloud_user_id := OLD.soundcloud_user_id;
            NEW.username := OLD.username;
            NEW.created_at := OLD.created_at;
            NEW.oauth_app_id := OLD.oauth_app_id;
            NEW.access_token := OLD.access_token;
            NEW.refresh_token := OLD.refresh_token;
            NEW.expires_at := OLD.expires_at;
            NEW.scope := OLD.scope;
            NEW.token_v := OLD.token_v;
        END IF;
        IF core THEN
            NEW.refresh_generation := OLD.refresh_generation;
            NEW.refresh_failure_count := OLD.refresh_failure_count;
            NEW.refresh_lease_id := OLD.refresh_lease_id;
            NEW.refresh_lease_expires_at := OLD.refresh_lease_expires_at;
            NEW.last_refresh_attempt_at := OLD.last_refresh_attempt_at;
            NEW.last_refresh_success_at := OLD.last_refresh_success_at;
            NEW.last_refresh_error_kind := OLD.last_refresh_error_kind;
            NEW.last_refresh_error := OLD.last_refresh_error;
            NEW.retry_at := OLD.retry_at;
            NEW.refresh_rejection_count := OLD.refresh_rejection_count;
            NEW.first_refresh_rejection_at := OLD.first_refresh_rejection_at;
            NEW.updated_at := OLD.updated_at;
            NEW.login_v := OLD.login_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.soundcloud_connections t WHERE t.id = NEW.id;
        IF FOUND THEN
            IF (NOT core AND (NEW.refresh_generation, NEW.refresh_failure_count, NEW.refresh_lease_id, NEW.refresh_lease_expires_at, NEW.last_refresh_attempt_at, NEW.last_refresh_success_at, NEW.last_refresh_error_kind, NEW.last_refresh_error, NEW.retry_at, NEW.refresh_rejection_count, NEW.first_refresh_rejection_at, NEW.updated_at, NEW.login_v) IS DISTINCT FROM (o.refresh_generation, o.refresh_failure_count, o.refresh_lease_id, o.refresh_lease_expires_at, o.last_refresh_attempt_at, o.last_refresh_success_at, o.last_refresh_error_kind, o.last_refresh_error, o.retry_at, o.refresh_rejection_count, o.first_refresh_rejection_at, o.updated_at, o.login_v)) OR NEW.token_v > o.token_v OR (((NEW.token_v & 15) <> public.scd_node_ord() OR NEW.token_v = o.token_v) AND NEW.created_at < o.created_at) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.soundcloud_connections t SET
                    refresh_generation = CASE WHEN core THEN o.refresh_generation ELSE NEW.refresh_generation END,
                    refresh_failure_count = CASE WHEN core THEN o.refresh_failure_count ELSE NEW.refresh_failure_count END,
                    refresh_lease_id = CASE WHEN core THEN o.refresh_lease_id ELSE NEW.refresh_lease_id END,
                    refresh_lease_expires_at = CASE WHEN core THEN o.refresh_lease_expires_at ELSE NEW.refresh_lease_expires_at END,
                    last_refresh_attempt_at = CASE WHEN core THEN o.last_refresh_attempt_at ELSE NEW.last_refresh_attempt_at END,
                    last_refresh_success_at = CASE WHEN core THEN o.last_refresh_success_at ELSE NEW.last_refresh_success_at END,
                    last_refresh_error_kind = CASE WHEN core THEN o.last_refresh_error_kind ELSE NEW.last_refresh_error_kind END,
                    last_refresh_error = CASE WHEN core THEN o.last_refresh_error ELSE NEW.last_refresh_error END,
                    retry_at = CASE WHEN core THEN o.retry_at ELSE NEW.retry_at END,
                    refresh_rejection_count = CASE WHEN core THEN o.refresh_rejection_count ELSE NEW.refresh_rejection_count END,
                    first_refresh_rejection_at = CASE WHEN core THEN o.first_refresh_rejection_at ELSE NEW.first_refresh_rejection_at END,
                    updated_at = CASE WHEN core THEN o.updated_at ELSE NEW.updated_at END,
                    login_v = CASE WHEN core THEN o.login_v ELSE NEW.login_v END,
                    soundcloud_user_id = CASE WHEN NEW.token_v > o.token_v THEN NEW.soundcloud_user_id ELSE o.soundcloud_user_id END,
                    username = CASE WHEN NEW.token_v > o.token_v THEN NEW.username ELSE o.username END,
                    created_at = CASE WHEN ((NEW.token_v & 15) <> public.scd_node_ord() OR NEW.token_v = o.token_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.token_v > o.token_v THEN NEW.created_at ELSE o.created_at END,
                    oauth_app_id = CASE WHEN NEW.token_v > o.token_v THEN NEW.oauth_app_id ELSE o.oauth_app_id END,
                    access_token = CASE WHEN NEW.token_v > o.token_v THEN NEW.access_token ELSE o.access_token END,
                    refresh_token = CASE WHEN NEW.token_v > o.token_v THEN NEW.refresh_token ELSE o.refresh_token END,
                    expires_at = CASE WHEN NEW.token_v > o.token_v THEN NEW.expires_at ELSE o.expires_at END,
                    scope = CASE WHEN NEW.token_v > o.token_v THEN NEW.scope ELSE o.scope END,
                    token_v = greatest(o.token_v, NEW.token_v)
                WHERE t.id = o.id;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.soundcloud_connections (access_token, created_at, expires_at, first_refresh_rejection_at, id, last_refresh_attempt_at, last_refresh_error, last_refresh_error_kind, last_refresh_success_at, login_v, oauth_app_id, refresh_failure_count, refresh_generation, refresh_lease_expires_at, refresh_lease_id, refresh_rejection_count, refresh_token, retry_at, scope, soundcloud_user_id, token_v, updated_at, username)
        VALUES (NEW.access_token, NEW.created_at, NEW.expires_at, NEW.first_refresh_rejection_at, NEW.id, NEW.last_refresh_attempt_at, NEW.last_refresh_error, NEW.last_refresh_error_kind, NEW.last_refresh_success_at, NEW.login_v, NEW.oauth_app_id, NEW.refresh_failure_count, NEW.refresh_generation, NEW.refresh_lease_expires_at, NEW.refresh_lease_id, NEW.refresh_rejection_count, NEW.refresh_token, NEW.retry_at, NEW.scope, NEW.soundcloud_user_id, NEW.token_v, NEW.updated_at, NEW.username)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.soundcloud_connections still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.soundcloud_connections FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_soundcloud_connections();
ALTER TABLE public.soundcloud_connections ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_orphan_soundcloud_connections()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF NEW.oauth_app_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM public.oauth_apps p WHERE p.id = NEW.oauth_app_id) THEN
        PERFORM nextval('public.scd_orphan_soundcloud_connections'::regclass);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_orphan BEFORE INSERT OR UPDATE OF oauth_app_id ON public.soundcloud_connections FOR EACH ROW WHEN (public.scd_role() = 'core' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_orphan_soundcloud_connections();
ALTER TABLE public.soundcloud_connections ENABLE REPLICA TRIGGER zz_orphan;

CREATE OR REPLACE FUNCTION public.scd_purge_guard_soundcloud_connections()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    pv bigint;
BEGIN
    SELECT max(x.purged_v) INTO pv FROM public.repl_purges x WHERE x.tbl = 'soundcloud_connections' AND x.pk = jsonb_build_object('id', OLD.id);
    IF OLD.token_v > coalesce(pv, 0) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('soundcloud_connections', jsonb_build_object('id', OLD.id)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_soundcloud_connections'::regclass);
        RETURN NULL;
    END IF;
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_purge_guard BEFORE DELETE ON public.soundcloud_connections FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_purge_guard_soundcloud_connections();
ALTER TABLE public.soundcloud_connections ENABLE REPLICA TRIGGER zz_purge_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_soundcloud_connections()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.token_v := public.scd_hlc(greatest(NEW.token_v, (SELECT max(x.purged_v) FROM public.repl_purges x WHERE x.tbl = 'soundcloud_connections' AND x.pk = jsonb_build_object('id', NEW.id))));
        RETURN NEW;
    END IF;
    IF (NEW.soundcloud_user_id, NEW.username, NEW.created_at, NEW.oauth_app_id, NEW.access_token, NEW.refresh_token, NEW.expires_at, NEW.scope) IS DISTINCT FROM (OLD.soundcloud_user_id, OLD.username, OLD.created_at, OLD.oauth_app_id, OLD.access_token, OLD.refresh_token, OLD.expires_at, OLD.scope) AND NOT (coalesce(current_setting('scd.cmd', true), '') <> '' AND NEW.token_v > greatest(OLD.token_v, (SELECT max(x.purged_v) FROM public.repl_purges x WHERE x.tbl = 'soundcloud_connections' AND x.pk = jsonb_build_object('id', NEW.id)))) THEN
        NEW.token_v := public.scd_hlc(greatest(OLD.token_v, NEW.token_v, (SELECT max(x.purged_v) FROM public.repl_purges x WHERE x.tbl = 'soundcloud_connections' AND x.pk = jsonb_build_object('id', NEW.id))));
    ELSE
        NEW.token_v := greatest(OLD.token_v, NEW.token_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.soundcloud_connections FOR EACH ROW EXECUTE FUNCTION public.scd_ver_soundcloud_connections();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.soundcloud_connections FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.soundcloud_connections ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.star_track_tiers FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_star_track_tiers', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_purge_storage_event_repairs(k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
DECLARE
    p record;
BEGIN
    SELECT t.sc_track_id INTO p FROM public.storage_event_repairs t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    DELETE FROM public.storage_event_repairs t WHERE t.sc_track_id = (k ->> 'sc_track_id')::text;
END
$f$;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.storage_event_repairs FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_storage_event_repairs', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.storage_event_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_storage_event_state', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.subscription_snapshot_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_subscription_snapshot_state', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_merge_subscriptions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.exp_date := OLD.exp_date;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.subscriptions t WHERE t.user_urn = NEW.user_urn;
        IF FOUND THEN
            IF NEW.row_v > o.row_v THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.subscriptions t SET
                    exp_date = CASE WHEN NEW.row_v > o.row_v THEN NEW.exp_date ELSE o.exp_date END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE t.user_urn = o.user_urn;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'subscriptions' AND x.pk = jsonb_build_object('user_urn', NEW.user_urn));
        IF tv IS NOT NULL AND NEW.row_v <= tv THEN
            INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'subscriptions', 'pk', jsonb_build_object('user_urn', NEW.user_urn), 'v', tv)) ON CONFLICT DO NOTHING;
            PERFORM nextval('public.scd_resend_subscriptions'::regclass);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.subscriptions (exp_date, row_v, user_urn)
        VALUES (NEW.exp_date, NEW.row_v, NEW.user_urn)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.subscriptions still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.subscriptions FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_subscriptions();
ALTER TABLE public.subscriptions ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_subscriptions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('subscriptions', jsonb_build_object('user_urn', OLD.user_urn), OLD.row_v, now());
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb BEFORE DELETE ON public.subscriptions FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_subscriptions();
CREATE OR REPLACE FUNCTION public.scd_erase_subscriptions(k jsonb, upto bigint)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    DELETE FROM public.subscriptions t WHERE t.user_urn = (k ->> 'user_urn')::text AND t.row_v <= upto;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
    IF EXISTS (SELECT 1 FROM public.subscriptions t WHERE t.user_urn = (k ->> 'user_urn')::text) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('subscriptions', k) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_subscriptions'::regclass);
    END IF;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_tomb_check_subscriptions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'subscriptions' AND x.pk = jsonb_build_object('user_urn', NEW.user_urn));
    IF tv IS NOT NULL AND NEW.row_v <= tv THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'subscriptions', 'pk', jsonb_build_object('user_urn', NEW.user_urn), 'v', tv)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_subscriptions'::regclass);
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.subscriptions FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_subscriptions();
ALTER TABLE public.subscriptions ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_tomb_guard_subscriptions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN OLD;
    END IF;
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'subscriptions' AND x.pk = jsonb_build_object('user_urn', OLD.user_urn));
    IF tv IS NULL OR OLD.row_v <= tv THEN
        RETURN OLD;
    END IF;
    INSERT INTO public.repl_resend (tbl, pk) VALUES ('subscriptions', jsonb_build_object('user_urn', OLD.user_urn)) ON CONFLICT DO NOTHING;
    PERFORM nextval('public.scd_resend_subscriptions'::regclass);
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_guard BEFORE DELETE ON public.subscriptions FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_guard_subscriptions();
ALTER TABLE public.subscriptions ENABLE REPLICA TRIGGER zz_tomb_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_subscriptions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(greatest(NEW.row_v, (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'subscriptions' AND x.pk = jsonb_build_object('user_urn', NEW.user_urn))));
        RETURN NEW;
    END IF;
    IF NEW.exp_date IS DISTINCT FROM OLD.exp_date THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.subscriptions FOR EACH ROW EXECUTE FUNCTION public.scd_ver_subscriptions();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.subscriptions FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.subscriptions ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

DROP FUNCTION IF EXISTS public.scd_purge_intent_sync_queue(jsonb);
CREATE OR REPLACE FUNCTION public.scd_purge_intent_sync_queue(k jsonb, seen bigint)
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
    IF NOT FOUND OR v IS DISTINCT FROM seen THEN
        RETURN false;
    END IF;
    INSERT INTO public.repl_purges (tbl, pk, purged_v) VALUES ('sync_queue', key, v);
    DELETE FROM public.sync_queue t WHERE t.id = (k ->> 'id')::uuid;
    RETURN true;
END
$f$;

CREATE OR REPLACE TRIGGER zz_guard_intent BEFORE INSERT OR UPDATE OF action_type, cancelled, created_at, generation, intent_v, payload, target_urn, user_id ON public.sync_queue FOR EACH STATEMENT WHEN (public.scd_role() = 'core') EXECUTE FUNCTION public.scd_guard('public.scd_guard_intent_sync_queue', 'mirror', 'migration', 'repl', 'api');

CREATE OR REPLACE TRIGGER zz_guard_result BEFORE UPDATE OF dead, exec_generation, failed_at, last_error, lease_generation, lease_id, locked_at, next_run_at, remote_attempted_generation, remote_completed_generation, remote_result, retry_count ON public.sync_queue FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_result_sync_queue', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_merge_sync_queue()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
    pv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    pv := (SELECT max(x.purged_v) FROM public.repl_purges x WHERE x.tbl = 'sync_queue' AND x.pk = jsonb_build_object('id', NEW.id));
    IF pv IS NOT NULL AND NEW.intent_v <= pv THEN
        PERFORM nextval('public.scd_purge_block_sync_queue'::regclass);
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.intent_v < OLD.intent_v OR (core AND NEW.intent_v = OLD.intent_v) THEN
            NEW.user_id := OLD.user_id;
            NEW.action_type := OLD.action_type;
            NEW.target_urn := OLD.target_urn;
            NEW.created_at := OLD.created_at;
            NEW.payload := OLD.payload;
            NEW.generation := OLD.generation;
            NEW.cancelled := OLD.cancelled;
            NEW.intent_v := OLD.intent_v;
        END IF;
        IF core THEN
            NEW.locked_at := OLD.locked_at;
            NEW.lease_id := OLD.lease_id;
            NEW.lease_generation := OLD.lease_generation;
            NEW.retry_count := OLD.retry_count;
            NEW.last_error := OLD.last_error;
            NEW.next_run_at := OLD.next_run_at;
            NEW.dead := OLD.dead;
            NEW.failed_at := OLD.failed_at;
            NEW.remote_attempted_generation := OLD.remote_attempted_generation;
            NEW.remote_completed_generation := OLD.remote_completed_generation;
            NEW.remote_result := OLD.remote_result;
            NEW.exec_generation := OLD.exec_generation;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.sync_queue t WHERE t.id = NEW.id;
        IF FOUND THEN
            IF NEW.intent_v > o.intent_v OR (((NEW.intent_v & 15) <> public.scd_node_ord() OR NEW.intent_v = o.intent_v) AND NEW.created_at < o.created_at) OR (NOT core AND (NEW.locked_at, NEW.lease_id, NEW.lease_generation, NEW.retry_count, NEW.last_error, NEW.next_run_at, NEW.dead, NEW.failed_at, NEW.remote_attempted_generation, NEW.remote_completed_generation, NEW.remote_result, NEW.exec_generation) IS DISTINCT FROM (o.locked_at, o.lease_id, o.lease_generation, o.retry_count, o.last_error, o.next_run_at, o.dead, o.failed_at, o.remote_attempted_generation, o.remote_completed_generation, o.remote_result, o.exec_generation)) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.sync_queue t SET
                    user_id = CASE WHEN NEW.intent_v > o.intent_v THEN NEW.user_id ELSE o.user_id END,
                    action_type = CASE WHEN NEW.intent_v > o.intent_v THEN NEW.action_type ELSE o.action_type END,
                    target_urn = CASE WHEN NEW.intent_v > o.intent_v THEN NEW.target_urn ELSE o.target_urn END,
                    created_at = CASE WHEN ((NEW.intent_v & 15) <> public.scd_node_ord() OR NEW.intent_v = o.intent_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.intent_v > o.intent_v THEN NEW.created_at ELSE o.created_at END,
                    payload = CASE WHEN NEW.intent_v > o.intent_v THEN NEW.payload ELSE o.payload END,
                    generation = CASE WHEN NEW.intent_v > o.intent_v THEN NEW.generation ELSE o.generation END,
                    cancelled = CASE WHEN NEW.intent_v > o.intent_v THEN NEW.cancelled ELSE o.cancelled END,
                    intent_v = greatest(o.intent_v, NEW.intent_v),
                    locked_at = CASE WHEN core THEN o.locked_at ELSE NEW.locked_at END,
                    lease_id = CASE WHEN core THEN o.lease_id ELSE NEW.lease_id END,
                    lease_generation = CASE WHEN core THEN o.lease_generation ELSE NEW.lease_generation END,
                    retry_count = CASE WHEN core THEN o.retry_count ELSE NEW.retry_count END,
                    last_error = CASE WHEN core THEN o.last_error ELSE NEW.last_error END,
                    next_run_at = CASE WHEN core THEN o.next_run_at ELSE NEW.next_run_at END,
                    dead = CASE WHEN core THEN o.dead ELSE NEW.dead END,
                    failed_at = CASE WHEN core THEN o.failed_at ELSE NEW.failed_at END,
                    remote_attempted_generation = CASE WHEN core THEN o.remote_attempted_generation ELSE NEW.remote_attempted_generation END,
                    remote_completed_generation = CASE WHEN core THEN o.remote_completed_generation ELSE NEW.remote_completed_generation END,
                    remote_result = CASE WHEN core THEN o.remote_result ELSE NEW.remote_result END,
                    exec_generation = CASE WHEN core THEN o.exec_generation ELSE NEW.exec_generation END
                WHERE t.id = o.id;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        IF (((NEW.action_type)::text <> 'comment'::text)) THEN
            SELECT t.* INTO o FROM public.sync_queue t WHERE (t.user_id, t.action_type, t.target_urn) = (NEW.user_id, NEW.action_type, NEW.target_urn) AND (((t.action_type)::text <> 'comment'::text));
            IF FOUND THEN
                PERFORM nextval('public.scd_conflict_sync_queue'::regclass);
                IF NEW.intent_v > o.intent_v OR (((NEW.intent_v & 15) <> public.scd_node_ord() OR NEW.intent_v = o.intent_v) AND NEW.created_at < o.created_at) OR (NOT core AND (NEW.locked_at, NEW.lease_id, NEW.lease_generation, NEW.retry_count, NEW.last_error, NEW.next_run_at, NEW.dead, NEW.failed_at, NEW.remote_attempted_generation, NEW.remote_completed_generation, NEW.remote_result, NEW.exec_generation) IS DISTINCT FROM (o.locked_at, o.lease_id, o.lease_generation, o.retry_count, o.last_error, o.next_run_at, o.dead, o.failed_at, o.remote_attempted_generation, o.remote_completed_generation, o.remote_result, o.exec_generation)) THEN
                    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                    UPDATE public.sync_queue t SET
                        user_id = CASE WHEN NEW.intent_v > o.intent_v THEN NEW.user_id ELSE o.user_id END,
                        action_type = CASE WHEN NEW.intent_v > o.intent_v THEN NEW.action_type ELSE o.action_type END,
                        target_urn = CASE WHEN NEW.intent_v > o.intent_v THEN NEW.target_urn ELSE o.target_urn END,
                        created_at = CASE WHEN ((NEW.intent_v & 15) <> public.scd_node_ord() OR NEW.intent_v = o.intent_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.intent_v > o.intent_v THEN NEW.created_at ELSE o.created_at END,
                        payload = CASE WHEN NEW.intent_v > o.intent_v THEN NEW.payload ELSE o.payload END,
                        generation = CASE WHEN NEW.intent_v > o.intent_v THEN NEW.generation ELSE o.generation END,
                        cancelled = CASE WHEN NEW.intent_v > o.intent_v THEN NEW.cancelled ELSE o.cancelled END,
                        intent_v = greatest(o.intent_v, NEW.intent_v),
                        locked_at = CASE WHEN core THEN o.locked_at ELSE NEW.locked_at END,
                        lease_id = CASE WHEN core THEN o.lease_id ELSE NEW.lease_id END,
                        lease_generation = CASE WHEN core THEN o.lease_generation ELSE NEW.lease_generation END,
                        retry_count = CASE WHEN core THEN o.retry_count ELSE NEW.retry_count END,
                        last_error = CASE WHEN core THEN o.last_error ELSE NEW.last_error END,
                        next_run_at = CASE WHEN core THEN o.next_run_at ELSE NEW.next_run_at END,
                        dead = CASE WHEN core THEN o.dead ELSE NEW.dead END,
                        failed_at = CASE WHEN core THEN o.failed_at ELSE NEW.failed_at END,
                        remote_attempted_generation = CASE WHEN core THEN o.remote_attempted_generation ELSE NEW.remote_attempted_generation END,
                        remote_completed_generation = CASE WHEN core THEN o.remote_completed_generation ELSE NEW.remote_completed_generation END,
                        remote_result = CASE WHEN core THEN o.remote_result ELSE NEW.remote_result END,
                        exec_generation = CASE WHEN core THEN o.exec_generation ELSE NEW.exec_generation END
                    WHERE t.id = o.id;
                    GET DIAGNOSTICS n = ROW_COUNT;
                    PERFORM pg_catalog.set_config('scd.merging', '', true);
                    IF n = 0 THEN
                        CONTINUE;
                    END IF;
                END IF;
                RETURN NULL;
            END IF;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.sync_queue (action_type, cancelled, created_at, dead, exec_generation, failed_at, generation, id, intent_v, last_error, lease_generation, lease_id, locked_at, next_run_at, payload, remote_attempted_generation, remote_completed_generation, remote_result, retry_count, target_urn, user_id)
        VALUES (NEW.action_type, NEW.cancelled, NEW.created_at, NEW.dead, NEW.exec_generation, NEW.failed_at, NEW.generation, NEW.id, NEW.intent_v, NEW.last_error, NEW.lease_generation, NEW.lease_id, NEW.locked_at, NEW.next_run_at, NEW.payload, NEW.remote_attempted_generation, NEW.remote_completed_generation, NEW.remote_result, NEW.retry_count, NEW.target_urn, NEW.user_id)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.sync_queue still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.sync_queue FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_sync_queue();
ALTER TABLE public.sync_queue ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_purge_guard_sync_queue()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    pv bigint;
BEGIN
    SELECT max(x.purged_v) INTO pv FROM public.repl_purges x WHERE x.tbl = 'sync_queue' AND x.pk = jsonb_build_object('id', OLD.id);
    IF OLD.intent_v > coalesce(pv, 0) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('sync_queue', jsonb_build_object('id', OLD.id)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_sync_queue'::regclass);
        RETURN NULL;
    END IF;
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_purge_guard BEFORE DELETE ON public.sync_queue FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_purge_guard_sync_queue();
ALTER TABLE public.sync_queue ENABLE REPLICA TRIGGER zz_purge_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_sync_queue()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF public.scd_role() = 'api' OR current_setting('scd.writer', true) = 'api' THEN
            NEW.intent_v := public.scd_hlc(greatest(NEW.intent_v, (SELECT max(x.purged_v) FROM public.repl_purges x WHERE x.tbl = 'sync_queue' AND x.pk = jsonb_build_object('id', NEW.id))));
        END IF;
        RETURN NEW;
    END IF;
    IF (NEW.user_id, NEW.action_type, NEW.target_urn, NEW.created_at, NEW.payload, NEW.generation, NEW.cancelled) IS DISTINCT FROM (OLD.user_id, OLD.action_type, OLD.target_urn, OLD.created_at, OLD.payload, OLD.generation, OLD.cancelled) THEN
        NEW.intent_v := public.scd_hlc(greatest(OLD.intent_v, NEW.intent_v, (SELECT max(x.purged_v) FROM public.repl_purges x WHERE x.tbl = 'sync_queue' AND x.pk = jsonb_build_object('id', NEW.id))));
    ELSE
        NEW.intent_v := greatest(OLD.intent_v, NEW.intent_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.sync_queue FOR EACH ROW EXECUTE FUNCTION public.scd_ver_sync_queue();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.sync_queue FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.sync_queue ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.sync_queue_action_pauses FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_sync_queue_action_pauses', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_sync_queue_action_pauses()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'sync_queue_action_pauses' AND p.pk = jsonb_build_object('oauth_app_id', OLD.oauth_app_id, 'action_type', OLD.action_type)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.oauth_app_id, NEW.action_type) IS DISTINCT FROM (OLD.oauth_app_id, OLD.action_type) THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'sync_queue_action_pauses' AND p.pk = jsonb_build_object('oauth_app_id', OLD.oauth_app_id, 'action_type', OLD.action_type)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'sync_queue_action_pauses' AND p.pk = jsonb_build_object('oauth_app_id', NEW.oauth_app_id, 'action_type', NEW.action_type)) THEN
                    INSERT INTO public.sync_queue_action_pauses (action_type, oauth_app_id, paused_until, reason, strikes, updated_at) VALUES (NEW.action_type, NEW.oauth_app_id, NEW.paused_until, NEW.reason, NEW.strikes, NEW.updated_at);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'sync_queue_action_pauses' AND p.pk = jsonb_build_object('oauth_app_id', NEW.oauth_app_id, 'action_type', NEW.action_type)) THEN
                DELETE FROM public.sync_queue_action_pauses t WHERE (t.oauth_app_id, t.action_type) = (OLD.oauth_app_id, OLD.action_type);
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'sync_queue_action_pauses' AND p.pk = jsonb_build_object('oauth_app_id', NEW.oauth_app_id, 'action_type', NEW.action_type)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.sync_queue_action_pauses t WHERE (t.oauth_app_id, t.action_type) = (NEW.oauth_app_id, NEW.action_type) FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'sync_queue_action_pauses' AND p.pk = jsonb_build_object('oauth_app_id', NEW.oauth_app_id, 'action_type', NEW.action_type)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.sync_queue_action_pauses t SET paused_until = NEW.paused_until, reason = NEW.reason, strikes = NEW.strikes, updated_at = NEW.updated_at WHERE (t.oauth_app_id, t.action_type) = (NEW.oauth_app_id, NEW.action_type);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.sync_queue_action_pauses (action_type, oauth_app_id, paused_until, reason, strikes, updated_at)
        VALUES (NEW.action_type, NEW.oauth_app_id, NEW.paused_until, NEW.reason, NEW.strikes, NEW.updated_at)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.sync_queue_action_pauses still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.sync_queue_action_pauses FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_sync_queue_action_pauses();
ALTER TABLE public.sync_queue_action_pauses ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.sync_queue_action_pauses FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.sync_queue_action_pauses ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.taste_model_versions FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_taste_model_versions', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_taste_model_versions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
    k0 boolean;
    k1 boolean;
    k2 boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'taste_model_versions' AND p.pk = jsonb_build_object('version', OLD.version)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.version IS DISTINCT FROM OLD.version THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'taste_model_versions' AND p.pk = jsonb_build_object('version', OLD.version)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'taste_model_versions' AND p.pk = jsonb_build_object('version', NEW.version)) THEN
                    INSERT INTO public.taste_model_versions (active, applied_at, collection, created_at, dim, input_object, items_count, metrics, pooling, refreshed_through, trained_at, users_count, version) VALUES (NEW.active, NEW.applied_at, NEW.collection, NEW.created_at, NEW.dim, NEW.input_object, NEW.items_count, NEW.metrics, NEW.pooling, NEW.refreshed_through, NEW.trained_at, NEW.users_count, NEW.version);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'taste_model_versions' AND p.pk = jsonb_build_object('version', NEW.version)) THEN
                DELETE FROM public.taste_model_versions t WHERE t.version = OLD.version;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'taste_model_versions' AND p.pk = jsonb_build_object('version', NEW.version)) THEN
                RETURN NULL;
            END IF;
        END IF;
        k0 := NEW.collection IS DISTINCT FROM OLD.collection;
        k1 := NEW.input_object IS DISTINCT FROM OLD.input_object;
        k2 := (((true)), (NEW.active)) IS DISTINCT FROM (((true)), (OLD.active));
        IF k0 THEN
            FOR r IN SELECT t.version FROM public.taste_model_versions t WHERE t.collection = NEW.collection AND t.version IS DISTINCT FROM NEW.version LOOP
                PERFORM public.scd_purge_taste_model_versions(jsonb_build_object('version', r.version));
            END LOOP;
        END IF;
        IF k1 THEN
            FOR r IN SELECT t.version FROM public.taste_model_versions t WHERE t.input_object = NEW.input_object AND t.version IS DISTINCT FROM NEW.version LOOP
                PERFORM public.scd_purge_taste_model_versions(jsonb_build_object('version', r.version));
            END LOOP;
        END IF;
        IF k2 AND (NEW.active) THEN
            FOR r IN SELECT t.version FROM public.taste_model_versions t WHERE ((true)) = ((true)) AND (t.active) AND t.version IS DISTINCT FROM NEW.version LOOP
                PERFORM public.scd_purge_taste_model_versions(jsonb_build_object('version', r.version));
            END LOOP;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.taste_model_versions t WHERE t.version = NEW.version FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'taste_model_versions' AND p.pk = jsonb_build_object('version', NEW.version)) THEN
            RETURN NULL;
        END IF;
        FOR r IN SELECT t.version FROM public.taste_model_versions t WHERE t.collection = NEW.collection AND t.version IS DISTINCT FROM NEW.version LOOP
            PERFORM public.scd_purge_taste_model_versions(jsonb_build_object('version', r.version));
        END LOOP;
        FOR r IN SELECT t.version FROM public.taste_model_versions t WHERE t.input_object = NEW.input_object AND t.version IS DISTINCT FROM NEW.version LOOP
            PERFORM public.scd_purge_taste_model_versions(jsonb_build_object('version', r.version));
        END LOOP;
        IF (NEW.active) THEN
            FOR r IN SELECT t.version FROM public.taste_model_versions t WHERE ((true)) = ((true)) AND (t.active) AND t.version IS DISTINCT FROM NEW.version LOOP
                PERFORM public.scd_purge_taste_model_versions(jsonb_build_object('version', r.version));
            END LOOP;
        END IF;
        IF held THEN
            UPDATE public.taste_model_versions t SET active = NEW.active, applied_at = NEW.applied_at, collection = NEW.collection, created_at = NEW.created_at, dim = NEW.dim, input_object = NEW.input_object, items_count = NEW.items_count, metrics = NEW.metrics, pooling = NEW.pooling, refreshed_through = NEW.refreshed_through, trained_at = NEW.trained_at, users_count = NEW.users_count WHERE t.version = NEW.version;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.taste_model_versions (active, applied_at, collection, created_at, dim, input_object, items_count, metrics, pooling, refreshed_through, trained_at, users_count, version)
        VALUES (NEW.active, NEW.applied_at, NEW.collection, NEW.created_at, NEW.dim, NEW.input_object, NEW.items_count, NEW.metrics, NEW.pooling, NEW.refreshed_through, NEW.trained_at, NEW.users_count, NEW.version)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.taste_model_versions still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.taste_model_versions FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_taste_model_versions();
ALTER TABLE public.taste_model_versions ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.taste_model_versions FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.taste_model_versions ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.taste_schedule FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_taste_schedule', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_merge_track_artist_blocks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.created_at := OLD.created_at;
            NEW.note := OLD.note;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.track_artist_blocks t WHERE (t.track_id, t.artist_id) = (NEW.track_id, NEW.artist_id);
        IF FOUND THEN
            IF NEW.row_v > o.row_v OR (((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) AND NEW.created_at < o.created_at) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.track_artist_blocks t SET
                    created_at = CASE WHEN ((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.row_v > o.row_v THEN NEW.created_at ELSE o.created_at END,
                    note = CASE WHEN NEW.row_v > o.row_v THEN NEW.note ELSE o.note END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE (t.track_id, t.artist_id) = (o.track_id, o.artist_id);
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'track_artist_blocks' AND x.pk = jsonb_build_object('track_id', NEW.track_id, 'artist_id', NEW.artist_id));
        IF tv IS NOT NULL AND NEW.row_v <= tv THEN
            INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'track_artist_blocks', 'pk', jsonb_build_object('track_id', NEW.track_id, 'artist_id', NEW.artist_id), 'v', tv)) ON CONFLICT DO NOTHING;
            PERFORM nextval('public.scd_resend_track_artist_blocks'::regclass);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.track_artist_blocks (artist_id, created_at, note, row_v, track_id)
        VALUES (NEW.artist_id, NEW.created_at, NEW.note, NEW.row_v, NEW.track_id)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.track_artist_blocks still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.track_artist_blocks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_track_artist_blocks();
ALTER TABLE public.track_artist_blocks ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_orphan_track_artist_blocks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF NEW.artist_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM public.artists p WHERE p.id = NEW.artist_id) THEN
        PERFORM nextval('public.scd_orphan_track_artist_blocks'::regclass);
    END IF;
    IF NEW.track_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM public.tracks p WHERE p.id = NEW.track_id) THEN
        PERFORM nextval('public.scd_orphan_track_artist_blocks'::regclass);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_orphan BEFORE INSERT OR UPDATE OF artist_id, track_id ON public.track_artist_blocks FOR EACH ROW WHEN (public.scd_role() = 'core' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_orphan_track_artist_blocks();
ALTER TABLE public.track_artist_blocks ENABLE REPLICA TRIGGER zz_orphan;

CREATE OR REPLACE FUNCTION public.scd_tomb_track_artist_blocks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('track_artist_blocks', jsonb_build_object('track_id', OLD.track_id, 'artist_id', OLD.artist_id), OLD.row_v, now());
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb BEFORE DELETE ON public.track_artist_blocks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_track_artist_blocks();
CREATE OR REPLACE FUNCTION public.scd_erase_track_artist_blocks(k jsonb, upto bigint)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    DELETE FROM public.track_artist_blocks t WHERE t.track_id = (k ->> 'track_id')::uuid AND t.artist_id = (k ->> 'artist_id')::uuid AND t.row_v <= upto;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
    IF EXISTS (SELECT 1 FROM public.track_artist_blocks t WHERE t.track_id = (k ->> 'track_id')::uuid AND t.artist_id = (k ->> 'artist_id')::uuid) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('track_artist_blocks', k) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_track_artist_blocks'::regclass);
    END IF;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_tomb_check_track_artist_blocks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'track_artist_blocks' AND x.pk = jsonb_build_object('track_id', NEW.track_id, 'artist_id', NEW.artist_id));
    IF tv IS NOT NULL AND NEW.row_v <= tv THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'track_artist_blocks', 'pk', jsonb_build_object('track_id', NEW.track_id, 'artist_id', NEW.artist_id), 'v', tv)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_track_artist_blocks'::regclass);
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.track_artist_blocks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_track_artist_blocks();
ALTER TABLE public.track_artist_blocks ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_tomb_guard_track_artist_blocks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN OLD;
    END IF;
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'track_artist_blocks' AND x.pk = jsonb_build_object('track_id', OLD.track_id, 'artist_id', OLD.artist_id));
    IF tv IS NULL OR OLD.row_v <= tv THEN
        RETURN OLD;
    END IF;
    INSERT INTO public.repl_resend (tbl, pk) VALUES ('track_artist_blocks', jsonb_build_object('track_id', OLD.track_id, 'artist_id', OLD.artist_id)) ON CONFLICT DO NOTHING;
    PERFORM nextval('public.scd_resend_track_artist_blocks'::regclass);
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_guard BEFORE DELETE ON public.track_artist_blocks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_guard_track_artist_blocks();
ALTER TABLE public.track_artist_blocks ENABLE REPLICA TRIGGER zz_tomb_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_track_artist_blocks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(greatest(NEW.row_v, (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'track_artist_blocks' AND x.pk = jsonb_build_object('track_id', NEW.track_id, 'artist_id', NEW.artist_id))));
        RETURN NEW;
    END IF;
    IF (NEW.created_at, NEW.note) IS DISTINCT FROM (OLD.created_at, OLD.note) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.track_artist_blocks FOR EACH ROW EXECUTE FUNCTION public.scd_ver_track_artist_blocks();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.track_artist_blocks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.track_artist_blocks ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.track_artists FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_track_artists', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_track_artists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'track_artists' AND p.pk = jsonb_build_object('track_id', OLD.track_id, 'artist_id', OLD.artist_id, 'role', OLD.role)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.track_id, NEW.artist_id, NEW.role) IS DISTINCT FROM (OLD.track_id, OLD.artist_id, OLD.role) THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'track_artists' AND p.pk = jsonb_build_object('track_id', OLD.track_id, 'artist_id', OLD.artist_id, 'role', OLD.role)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'track_artists' AND p.pk = jsonb_build_object('track_id', NEW.track_id, 'artist_id', NEW.artist_id, 'role', NEW.role)) THEN
                    INSERT INTO public.track_artists (artist_id, confidence, evidence, "position", role, source, track_id) VALUES (NEW.artist_id, NEW.confidence, NEW.evidence, NEW."position", NEW.role, NEW.source, NEW.track_id);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'track_artists' AND p.pk = jsonb_build_object('track_id', NEW.track_id, 'artist_id', NEW.artist_id, 'role', NEW.role)) THEN
                DELETE FROM public.track_artists t WHERE (t.track_id, t.artist_id, t.role) = (OLD.track_id, OLD.artist_id, OLD.role);
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'track_artists' AND p.pk = jsonb_build_object('track_id', NEW.track_id, 'artist_id', NEW.artist_id, 'role', NEW.role)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.track_artists t WHERE (t.track_id, t.artist_id, t.role) = (NEW.track_id, NEW.artist_id, NEW.role) FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'track_artists' AND p.pk = jsonb_build_object('track_id', NEW.track_id, 'artist_id', NEW.artist_id, 'role', NEW.role)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.track_artists t SET confidence = NEW.confidence, evidence = NEW.evidence, "position" = NEW."position", source = NEW.source WHERE (t.track_id, t.artist_id, t.role) = (NEW.track_id, NEW.artist_id, NEW.role);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.track_artists (artist_id, confidence, evidence, "position", role, source, track_id)
        VALUES (NEW.artist_id, NEW.confidence, NEW.evidence, NEW."position", NEW.role, NEW.source, NEW.track_id)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.track_artists still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.track_artists FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_track_artists();
ALTER TABLE public.track_artists ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.track_artists FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.track_artists ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.track_comments FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_track_comments', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_track_comments()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
    k0 boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'track_comments' AND p.pk = jsonb_build_object('id', OLD.id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.id IS DISTINCT FROM OLD.id THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'track_comments' AND p.pk = jsonb_build_object('id', OLD.id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'track_comments' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                    INSERT INTO public.track_comments (body, created_at, id, sc_comment_id, sc_created_at, sc_track_id, synced_at, track_position_ms, user_urn) VALUES (NEW.body, NEW.created_at, NEW.id, NEW.sc_comment_id, NEW.sc_created_at, NEW.sc_track_id, NEW.synced_at, NEW.track_position_ms, NEW.user_urn);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'track_comments' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                DELETE FROM public.track_comments t WHERE t.id = OLD.id;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'track_comments' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        k0 := NEW.sc_comment_id IS DISTINCT FROM OLD.sc_comment_id;
        IF k0 THEN
            FOR r IN SELECT t.id FROM public.track_comments t WHERE t.sc_comment_id = NEW.sc_comment_id AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_track_comments(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.track_comments t WHERE t.id = NEW.id FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'track_comments' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
            RETURN NULL;
        END IF;
        FOR r IN SELECT t.id FROM public.track_comments t WHERE t.sc_comment_id = NEW.sc_comment_id AND t.id IS DISTINCT FROM NEW.id LOOP
            PERFORM public.scd_purge_track_comments(jsonb_build_object('id', r.id));
        END LOOP;
        IF held THEN
            UPDATE public.track_comments t SET body = NEW.body, created_at = NEW.created_at, sc_comment_id = NEW.sc_comment_id, sc_created_at = NEW.sc_created_at, sc_track_id = NEW.sc_track_id, synced_at = NEW.synced_at, track_position_ms = NEW.track_position_ms, user_urn = NEW.user_urn WHERE t.id = NEW.id;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.track_comments (body, created_at, id, sc_comment_id, sc_created_at, sc_track_id, synced_at, track_position_ms, user_urn)
        VALUES (NEW.body, NEW.created_at, NEW.id, NEW.sc_comment_id, NEW.sc_created_at, NEW.sc_track_id, NEW.synced_at, NEW.track_position_ms, NEW.user_urn)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.track_comments still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.track_comments FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_track_comments();
ALTER TABLE public.track_comments ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.track_comments FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.track_comments ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.track_work_aliases FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_track_work_aliases', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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
    DELETE FROM public.storage_event_repairs c WHERE c.sc_track_id = p.sc_track_id;
    DELETE FROM public.storage_event_state c WHERE c.sc_track_id = p.sc_track_id;
    DELETE FROM public.track_artist_blocks c WHERE c.track_id = p.id;
    DELETE FROM public.track_artists c WHERE c.track_id = p.id;
    DELETE FROM public.track_work_aliases c WHERE c.track_id = p.id;
    UPDATE public.tracks c SET superseded_by = NULL WHERE c.superseded_by = p.id;
    UPDATE public.wanted_tracks c SET track_id = NULL WHERE c.track_id = p.id;
    DELETE FROM public.tracks t WHERE t.id = (k ->> 'id')::uuid;
END
$f$;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.tracks FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_tracks', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
    k0 boolean;
    k1 boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'tracks' AND p.pk = jsonb_build_object('id', OLD.id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.id IS DISTINCT FROM OLD.id THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'tracks' AND p.pk = jsonb_build_object('id', OLD.id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'tracks' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                    INSERT INTO public.tracks (album_id, album_position, artwork_url, audio_fingerprint, canonical_track_id, comments_count_sc, cover_of_artist_id, created_at, deleted_at, description, duration_ms, duration_resolve_attempts, duration_resolve_retry_at, enrich_attempts, enrich_confidence, enrich_error, enrich_locked_at, enrich_next_run_at, enrich_source, enrich_state, enriched_at, genius_song_id, genius_url, genre, hq_upgrade_attempts, hq_upgrade_last_at, hq_upgrade_pending, id, index_attempts, index_priority, index_state, indexed_at, is_cover, isrc, language, language_confidence, last_read_at, likes_count_sc, metadata_artist, needs_duration_resolve, permalink_url, pipeline_held, play_count_sc, primary_artist_id, quality_model_version, quality_score, recording_key, release_date, release_year, reposts_count_sc, s3_missing_at, s3_verified_at, sc_created_at, sc_desired, sc_last_modified, sc_metadata, sc_mutation_observation, sc_observation, sc_synced_at, sc_track_id, sc_write_confirmed, sharing, storage_attempts, storage_priority, storage_quality, storage_state, superseded_by, tags, title, title_normalized, transcribe_at, transcribe_state, updated_at, upload_kind, uploader_avatar_url, uploader_sc_user_id, uploader_urn, uploader_username, urn, waveform_url, work_key, work_normalizer_version) VALUES (NEW.album_id, NEW.album_position, NEW.artwork_url, NEW.audio_fingerprint, NEW.canonical_track_id, NEW.comments_count_sc, NEW.cover_of_artist_id, NEW.created_at, NEW.deleted_at, NEW.description, NEW.duration_ms, NEW.duration_resolve_attempts, NEW.duration_resolve_retry_at, NEW.enrich_attempts, NEW.enrich_confidence, NEW.enrich_error, NEW.enrich_locked_at, NEW.enrich_next_run_at, NEW.enrich_source, NEW.enrich_state, NEW.enriched_at, NEW.genius_song_id, NEW.genius_url, NEW.genre, NEW.hq_upgrade_attempts, NEW.hq_upgrade_last_at, NEW.hq_upgrade_pending, NEW.id, NEW.index_attempts, NEW.index_priority, NEW.index_state, NEW.indexed_at, NEW.is_cover, NEW.isrc, NEW.language, NEW.language_confidence, NEW.last_read_at, NEW.likes_count_sc, NEW.metadata_artist, NEW.needs_duration_resolve, NEW.permalink_url, NEW.pipeline_held, NEW.play_count_sc, NEW.primary_artist_id, NEW.quality_model_version, NEW.quality_score, NEW.recording_key, NEW.release_date, NEW.release_year, NEW.reposts_count_sc, NEW.s3_missing_at, NEW.s3_verified_at, NEW.sc_created_at, NEW.sc_desired, NEW.sc_last_modified, NEW.sc_metadata, NEW.sc_mutation_observation, NEW.sc_observation, NEW.sc_synced_at, NEW.sc_track_id, NEW.sc_write_confirmed, NEW.sharing, NEW.storage_attempts, NEW.storage_priority, NEW.storage_quality, NEW.storage_state, NEW.superseded_by, NEW.tags, NEW.title, NEW.title_normalized, NEW.transcribe_at, NEW.transcribe_state, NEW.updated_at, NEW.upload_kind, NEW.uploader_avatar_url, NEW.uploader_sc_user_id, NEW.uploader_urn, NEW.uploader_username, NEW.urn, NEW.waveform_url, NEW.work_key, NEW.work_normalizer_version);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'tracks' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                DELETE FROM public.tracks t WHERE t.id = OLD.id;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'tracks' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        k0 := NEW.sc_track_id IS DISTINCT FROM OLD.sc_track_id;
        k1 := NEW.urn IS DISTINCT FROM OLD.urn;
        IF k0 THEN
            FOR r IN SELECT t.id FROM public.tracks t WHERE t.sc_track_id = NEW.sc_track_id AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_tracks(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        IF k1 THEN
            FOR r IN SELECT t.id FROM public.tracks t WHERE t.urn = NEW.urn AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_tracks(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.tracks t WHERE t.id = NEW.id FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'tracks' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
            RETURN NULL;
        END IF;
        FOR r IN SELECT t.id FROM public.tracks t WHERE t.sc_track_id = NEW.sc_track_id AND t.id IS DISTINCT FROM NEW.id LOOP
            PERFORM public.scd_purge_tracks(jsonb_build_object('id', r.id));
        END LOOP;
        FOR r IN SELECT t.id FROM public.tracks t WHERE t.urn = NEW.urn AND t.id IS DISTINCT FROM NEW.id LOOP
            PERFORM public.scd_purge_tracks(jsonb_build_object('id', r.id));
        END LOOP;
        IF held THEN
            UPDATE public.tracks t SET album_id = NEW.album_id, album_position = NEW.album_position, artwork_url = NEW.artwork_url, audio_fingerprint = NEW.audio_fingerprint, canonical_track_id = NEW.canonical_track_id, comments_count_sc = NEW.comments_count_sc, cover_of_artist_id = NEW.cover_of_artist_id, created_at = NEW.created_at, deleted_at = NEW.deleted_at, description = NEW.description, duration_ms = NEW.duration_ms, duration_resolve_attempts = NEW.duration_resolve_attempts, duration_resolve_retry_at = NEW.duration_resolve_retry_at, enrich_attempts = NEW.enrich_attempts, enrich_confidence = NEW.enrich_confidence, enrich_error = NEW.enrich_error, enrich_locked_at = NEW.enrich_locked_at, enrich_next_run_at = NEW.enrich_next_run_at, enrich_source = NEW.enrich_source, enrich_state = NEW.enrich_state, enriched_at = NEW.enriched_at, genius_song_id = NEW.genius_song_id, genius_url = NEW.genius_url, genre = NEW.genre, hq_upgrade_attempts = NEW.hq_upgrade_attempts, hq_upgrade_last_at = NEW.hq_upgrade_last_at, hq_upgrade_pending = NEW.hq_upgrade_pending, index_attempts = NEW.index_attempts, index_priority = NEW.index_priority, index_state = NEW.index_state, indexed_at = NEW.indexed_at, is_cover = NEW.is_cover, isrc = NEW.isrc, language = NEW.language, language_confidence = NEW.language_confidence, likes_count_sc = NEW.likes_count_sc, metadata_artist = NEW.metadata_artist, needs_duration_resolve = NEW.needs_duration_resolve, permalink_url = NEW.permalink_url, pipeline_held = NEW.pipeline_held, play_count_sc = NEW.play_count_sc, primary_artist_id = NEW.primary_artist_id, quality_model_version = NEW.quality_model_version, quality_score = NEW.quality_score, recording_key = NEW.recording_key, release_date = NEW.release_date, release_year = NEW.release_year, reposts_count_sc = NEW.reposts_count_sc, s3_missing_at = NEW.s3_missing_at, s3_verified_at = NEW.s3_verified_at, sc_created_at = NEW.sc_created_at, sc_desired = NEW.sc_desired, sc_last_modified = NEW.sc_last_modified, sc_metadata = NEW.sc_metadata, sc_mutation_observation = NEW.sc_mutation_observation, sc_observation = NEW.sc_observation, sc_synced_at = NEW.sc_synced_at, sc_track_id = NEW.sc_track_id, sc_write_confirmed = NEW.sc_write_confirmed, sharing = NEW.sharing, storage_attempts = NEW.storage_attempts, storage_priority = NEW.storage_priority, storage_quality = NEW.storage_quality, storage_state = NEW.storage_state, superseded_by = NEW.superseded_by, tags = NEW.tags, title = NEW.title, title_normalized = NEW.title_normalized, transcribe_at = NEW.transcribe_at, transcribe_state = NEW.transcribe_state, updated_at = NEW.updated_at, upload_kind = NEW.upload_kind, uploader_avatar_url = NEW.uploader_avatar_url, uploader_sc_user_id = NEW.uploader_sc_user_id, uploader_urn = NEW.uploader_urn, uploader_username = NEW.uploader_username, urn = NEW.urn, waveform_url = NEW.waveform_url, work_key = NEW.work_key, work_normalizer_version = NEW.work_normalizer_version WHERE t.id = NEW.id;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.tracks (album_id, album_position, artwork_url, audio_fingerprint, canonical_track_id, comments_count_sc, cover_of_artist_id, created_at, deleted_at, description, duration_ms, duration_resolve_attempts, duration_resolve_retry_at, enrich_attempts, enrich_confidence, enrich_error, enrich_locked_at, enrich_next_run_at, enrich_source, enrich_state, enriched_at, genius_song_id, genius_url, genre, hq_upgrade_attempts, hq_upgrade_last_at, hq_upgrade_pending, id, index_attempts, index_priority, index_state, indexed_at, is_cover, isrc, language, language_confidence, last_read_at, likes_count_sc, metadata_artist, needs_duration_resolve, permalink_url, pipeline_held, play_count_sc, primary_artist_id, quality_model_version, quality_score, recording_key, release_date, release_year, reposts_count_sc, s3_missing_at, s3_verified_at, sc_created_at, sc_desired, sc_last_modified, sc_metadata, sc_mutation_observation, sc_observation, sc_synced_at, sc_track_id, sc_write_confirmed, sharing, storage_attempts, storage_priority, storage_quality, storage_state, superseded_by, tags, title, title_normalized, transcribe_at, transcribe_state, updated_at, upload_kind, uploader_avatar_url, uploader_sc_user_id, uploader_urn, uploader_username, urn, waveform_url, work_key, work_normalizer_version)
        VALUES (NEW.album_id, NEW.album_position, NEW.artwork_url, NEW.audio_fingerprint, NEW.canonical_track_id, NEW.comments_count_sc, NEW.cover_of_artist_id, NEW.created_at, NEW.deleted_at, NEW.description, NEW.duration_ms, NEW.duration_resolve_attempts, NEW.duration_resolve_retry_at, NEW.enrich_attempts, NEW.enrich_confidence, NEW.enrich_error, NEW.enrich_locked_at, NEW.enrich_next_run_at, NEW.enrich_source, NEW.enrich_state, NEW.enriched_at, NEW.genius_song_id, NEW.genius_url, NEW.genre, NEW.hq_upgrade_attempts, NEW.hq_upgrade_last_at, NEW.hq_upgrade_pending, NEW.id, NEW.index_attempts, NEW.index_priority, NEW.index_state, NEW.indexed_at, NEW.is_cover, NEW.isrc, NEW.language, NEW.language_confidence, NEW.last_read_at, NEW.likes_count_sc, NEW.metadata_artist, NEW.needs_duration_resolve, NEW.permalink_url, NEW.pipeline_held, NEW.play_count_sc, NEW.primary_artist_id, NEW.quality_model_version, NEW.quality_score, NEW.recording_key, NEW.release_date, NEW.release_year, NEW.reposts_count_sc, NEW.s3_missing_at, NEW.s3_verified_at, NEW.sc_created_at, NEW.sc_desired, NEW.sc_last_modified, NEW.sc_metadata, NEW.sc_mutation_observation, NEW.sc_observation, NEW.sc_synced_at, NEW.sc_track_id, NEW.sc_write_confirmed, NEW.sharing, NEW.storage_attempts, NEW.storage_priority, NEW.storage_quality, NEW.storage_state, NEW.superseded_by, NEW.tags, NEW.title, NEW.title_normalized, NEW.transcribe_at, NEW.transcribe_state, NEW.updated_at, NEW.upload_kind, NEW.uploader_avatar_url, NEW.uploader_sc_user_id, NEW.uploader_urn, NEW.uploader_username, NEW.urn, NEW.waveform_url, NEW.work_key, NEW.work_normalizer_version)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.tracks still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.tracks FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_tracks();
ALTER TABLE public.tracks ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.tracks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.tracks ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.transcription_sync_versions FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_transcription_sync_versions', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.transcription_wire_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_transcription_wire_state', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_merge_user_auras()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.aura_id := OLD.aura_id;
            NEW.custom_hex := OLD.custom_hex;
            NEW.updated_at := OLD.updated_at;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.user_auras t WHERE t.user_urn = NEW.user_urn;
        IF FOUND THEN
            IF NEW.row_v > o.row_v THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.user_auras t SET
                    aura_id = CASE WHEN NEW.row_v > o.row_v THEN NEW.aura_id ELSE o.aura_id END,
                    custom_hex = CASE WHEN NEW.row_v > o.row_v THEN NEW.custom_hex ELSE o.custom_hex END,
                    updated_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.updated_at ELSE o.updated_at END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE t.user_urn = o.user_urn;
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.user_auras (aura_id, custom_hex, row_v, updated_at, user_urn)
        VALUES (NEW.aura_id, NEW.custom_hex, NEW.row_v, NEW.updated_at, NEW.user_urn)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.user_auras still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.user_auras FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_user_auras();
ALTER TABLE public.user_auras ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_ver_user_auras()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.aura_id, NEW.custom_hex, NEW.updated_at) IS DISTINCT FROM (OLD.aura_id, OLD.custom_hex, OLD.updated_at) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.user_auras FOR EACH ROW EXECUTE FUNCTION public.scd_ver_user_auras();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_auras FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_auras ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_merge_user_blocked_artists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.avatar_url := OLD.avatar_url;
            NEW.created_at := OLD.created_at;
            NEW.name := OLD.name;
            NEW.sc_user_ids := OLD.sc_user_ids;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.user_blocked_artists t WHERE (t.sc_user_id, t.kind, t.target_id) = (NEW.sc_user_id, NEW.kind, NEW.target_id);
        IF FOUND THEN
            IF NEW.row_v > o.row_v OR (((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) AND NEW.created_at < o.created_at) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.user_blocked_artists t SET
                    avatar_url = CASE WHEN NEW.row_v > o.row_v THEN NEW.avatar_url ELSE o.avatar_url END,
                    created_at = CASE WHEN ((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.row_v > o.row_v THEN NEW.created_at ELSE o.created_at END,
                    name = CASE WHEN NEW.row_v > o.row_v THEN NEW.name ELSE o.name END,
                    sc_user_ids = CASE WHEN NEW.row_v > o.row_v THEN NEW.sc_user_ids ELSE o.sc_user_ids END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE (t.sc_user_id, t.kind, t.target_id) = (o.sc_user_id, o.kind, o.target_id);
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'user_blocked_artists' AND x.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id, 'kind', NEW.kind, 'target_id', NEW.target_id));
        IF tv IS NOT NULL AND NEW.row_v <= tv THEN
            INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'user_blocked_artists', 'pk', jsonb_build_object('sc_user_id', NEW.sc_user_id, 'kind', NEW.kind, 'target_id', NEW.target_id), 'v', tv)) ON CONFLICT DO NOTHING;
            PERFORM nextval('public.scd_resend_user_blocked_artists'::regclass);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.user_blocked_artists (avatar_url, created_at, kind, name, row_v, sc_user_id, sc_user_ids, target_id)
        VALUES (NEW.avatar_url, NEW.created_at, NEW.kind, NEW.name, NEW.row_v, NEW.sc_user_id, NEW.sc_user_ids, NEW.target_id)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.user_blocked_artists still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.user_blocked_artists FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_user_blocked_artists();
ALTER TABLE public.user_blocked_artists ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_user_blocked_artists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('user_blocked_artists', jsonb_build_object('sc_user_id', OLD.sc_user_id, 'kind', OLD.kind, 'target_id', OLD.target_id), OLD.row_v, now());
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb BEFORE DELETE ON public.user_blocked_artists FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_user_blocked_artists();
CREATE OR REPLACE FUNCTION public.scd_erase_user_blocked_artists(k jsonb, upto bigint)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    DELETE FROM public.user_blocked_artists t WHERE t.sc_user_id = (k ->> 'sc_user_id')::text AND t.kind = (k ->> 'kind')::text AND t.target_id = (k ->> 'target_id')::text AND t.row_v <= upto;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
    IF EXISTS (SELECT 1 FROM public.user_blocked_artists t WHERE t.sc_user_id = (k ->> 'sc_user_id')::text AND t.kind = (k ->> 'kind')::text AND t.target_id = (k ->> 'target_id')::text) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('user_blocked_artists', k) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_user_blocked_artists'::regclass);
    END IF;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_tomb_check_user_blocked_artists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'user_blocked_artists' AND x.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id, 'kind', NEW.kind, 'target_id', NEW.target_id));
    IF tv IS NOT NULL AND NEW.row_v <= tv THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'user_blocked_artists', 'pk', jsonb_build_object('sc_user_id', NEW.sc_user_id, 'kind', NEW.kind, 'target_id', NEW.target_id), 'v', tv)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_user_blocked_artists'::regclass);
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.user_blocked_artists FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_user_blocked_artists();
ALTER TABLE public.user_blocked_artists ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_tomb_guard_user_blocked_artists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN OLD;
    END IF;
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'user_blocked_artists' AND x.pk = jsonb_build_object('sc_user_id', OLD.sc_user_id, 'kind', OLD.kind, 'target_id', OLD.target_id));
    IF tv IS NULL OR OLD.row_v <= tv THEN
        RETURN OLD;
    END IF;
    INSERT INTO public.repl_resend (tbl, pk) VALUES ('user_blocked_artists', jsonb_build_object('sc_user_id', OLD.sc_user_id, 'kind', OLD.kind, 'target_id', OLD.target_id)) ON CONFLICT DO NOTHING;
    PERFORM nextval('public.scd_resend_user_blocked_artists'::regclass);
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_guard BEFORE DELETE ON public.user_blocked_artists FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_guard_user_blocked_artists();
ALTER TABLE public.user_blocked_artists ENABLE REPLICA TRIGGER zz_tomb_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_user_blocked_artists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(greatest(NEW.row_v, (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'user_blocked_artists' AND x.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id, 'kind', NEW.kind, 'target_id', NEW.target_id))));
        RETURN NEW;
    END IF;
    IF (NEW.avatar_url, NEW.created_at, NEW.name, NEW.sc_user_ids) IS DISTINCT FROM (OLD.avatar_url, OLD.created_at, OLD.name, OLD.sc_user_ids) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.user_blocked_artists FOR EACH ROW EXECUTE FUNCTION public.scd_ver_user_blocked_artists();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_blocked_artists FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_blocked_artists ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.user_collection_sync FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_user_collection_sync', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_skip_user_events()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        NEW.created_at := least(OLD.created_at, NEW.created_at);
        RETURN NEW;
    END IF;
    SELECT t.created_at INTO o FROM public.user_events t WHERE t.id = NEW.id;
    IF FOUND THEN
        IF NEW.created_at < o.created_at THEN
            PERFORM pg_catalog.set_config('scd.merging', 'on', true);
            UPDATE public.user_events t SET created_at = least(t.created_at, NEW.created_at) WHERE t.id = NEW.id;
            PERFORM pg_catalog.set_config('scd.merging', '', true);
        END IF;
        RETURN NULL;
    END IF;
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    INSERT INTO public.user_events (created_at, event_type, id, position_pct, sc_track_id, sc_user_id, weight)
    VALUES (NEW.created_at, NEW.event_type, NEW.id, NEW.position_pct, NEW.sc_track_id, NEW.sc_user_id, NEW.weight)
    ON CONFLICT DO NOTHING;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_skip BEFORE INSERT OR UPDATE ON public.user_events FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_skip_user_events();
ALTER TABLE public.user_events ENABLE REPLICA TRIGGER zz_skip;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_events FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_events ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.user_followers FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_user_followers', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_user_followers()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_followers' AND p.pk = jsonb_build_object('user_id', OLD.user_id, 'target_user_urn', OLD.target_user_urn)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.user_id, NEW.target_user_urn) IS DISTINCT FROM (OLD.user_id, OLD.target_user_urn) THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_followers' AND p.pk = jsonb_build_object('user_id', OLD.user_id, 'target_user_urn', OLD.target_user_urn)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_followers' AND p.pk = jsonb_build_object('user_id', NEW.user_id, 'target_user_urn', NEW.target_user_urn)) THEN
                    INSERT INTO public.user_followers (created_at, last_read_at, progress, synced_at, target_user_urn, user_id) VALUES (NEW.created_at, NEW.last_read_at, NEW.progress, NEW.synced_at, NEW.target_user_urn, NEW.user_id);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_followers' AND p.pk = jsonb_build_object('user_id', NEW.user_id, 'target_user_urn', NEW.target_user_urn)) THEN
                DELETE FROM public.user_followers t WHERE (t.user_id, t.target_user_urn) = (OLD.user_id, OLD.target_user_urn);
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_followers' AND p.pk = jsonb_build_object('user_id', NEW.user_id, 'target_user_urn', NEW.target_user_urn)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.user_followers t WHERE (t.user_id, t.target_user_urn) = (NEW.user_id, NEW.target_user_urn) FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_followers' AND p.pk = jsonb_build_object('user_id', NEW.user_id, 'target_user_urn', NEW.target_user_urn)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.user_followers t SET created_at = NEW.created_at, progress = NEW.progress, synced_at = NEW.synced_at WHERE (t.user_id, t.target_user_urn) = (NEW.user_id, NEW.target_user_urn);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.user_followers (created_at, last_read_at, progress, synced_at, target_user_urn, user_id)
        VALUES (NEW.created_at, NEW.last_read_at, NEW.progress, NEW.synced_at, NEW.target_user_urn, NEW.user_id)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.user_followers still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.user_followers FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_user_followers();
ALTER TABLE public.user_followers ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_followers FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_followers ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

DROP FUNCTION IF EXISTS public.scd_purge_intent_user_followings(jsonb);
CREATE OR REPLACE FUNCTION public.scd_purge_intent_user_followings(k jsonb, seen bigint)
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
    IF NOT FOUND OR v IS DISTINCT FROM seen THEN
        RETURN false;
    END IF;
    INSERT INTO public.repl_purges (tbl, pk, purged_v) VALUES ('user_followings', key, v);
    DELETE FROM public.user_followings t WHERE t.user_id = (k ->> 'user_id')::text AND t.target_user_urn = (k ->> 'target_user_urn')::text;
    RETURN true;
END
$f$;

CREATE OR REPLACE TRIGGER zz_guard_intent BEFORE INSERT OR UPDATE OF created_at, wanted_state, wanted_v ON public.user_followings FOR EACH STATEMENT WHEN (public.scd_role() = 'core') EXECUTE FUNCTION public.scd_guard('public.scd_guard_intent_user_followings', 'mirror', 'migration', 'repl', 'api');

CREATE OR REPLACE TRIGGER zz_guard_result BEFORE UPDATE OF progress, synced_at, synced_for_v ON public.user_followings FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_result_user_followings', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_merge_user_followings()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
    pv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    pv := (SELECT max(x.purged_v) FROM public.repl_purges x WHERE x.tbl = 'user_followings' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'target_user_urn', NEW.target_user_urn));
    IF pv IS NOT NULL AND NEW.wanted_v <= pv THEN
        PERFORM nextval('public.scd_purge_block_user_followings'::regclass);
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.wanted_v < OLD.wanted_v OR (core AND NEW.wanted_v = OLD.wanted_v) THEN
            NEW.wanted_state := OLD.wanted_state;
            NEW.created_at := OLD.created_at;
            NEW.wanted_v := OLD.wanted_v;
        END IF;
        IF core THEN
            NEW.progress := OLD.progress;
            NEW.synced_at := OLD.synced_at;
            NEW.synced_for_v := OLD.synced_for_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.user_followings t WHERE (t.user_id, t.target_user_urn) = (NEW.user_id, NEW.target_user_urn);
        IF FOUND THEN
            IF NEW.wanted_v > o.wanted_v OR (((NEW.wanted_v & 15) <> public.scd_node_ord() OR NEW.wanted_v = o.wanted_v) AND NEW.created_at < o.created_at) OR (NOT core AND (NEW.progress, NEW.synced_at, NEW.synced_for_v) IS DISTINCT FROM (o.progress, o.synced_at, o.synced_for_v)) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.user_followings t SET
                    wanted_state = CASE WHEN NEW.wanted_v > o.wanted_v THEN NEW.wanted_state ELSE o.wanted_state END,
                    created_at = CASE WHEN ((NEW.wanted_v & 15) <> public.scd_node_ord() OR NEW.wanted_v = o.wanted_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.wanted_v > o.wanted_v THEN NEW.created_at ELSE o.created_at END,
                    wanted_v = greatest(o.wanted_v, NEW.wanted_v),
                    progress = CASE WHEN core THEN o.progress ELSE NEW.progress END,
                    synced_at = CASE WHEN core THEN o.synced_at ELSE NEW.synced_at END,
                    synced_for_v = CASE WHEN core THEN o.synced_for_v ELSE NEW.synced_for_v END
                WHERE (t.user_id, t.target_user_urn) = (o.user_id, o.target_user_urn);
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.user_followings (created_at, last_read_at, progress, synced_at, synced_for_v, target_user_urn, user_id, wanted_state, wanted_v)
        VALUES (NEW.created_at, NEW.last_read_at, NEW.progress, NEW.synced_at, NEW.synced_for_v, NEW.target_user_urn, NEW.user_id, NEW.wanted_state, NEW.wanted_v)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.user_followings still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.user_followings FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_user_followings();
ALTER TABLE public.user_followings ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_purge_guard_user_followings()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    pv bigint;
BEGIN
    SELECT max(x.purged_v) INTO pv FROM public.repl_purges x WHERE x.tbl = 'user_followings' AND x.pk = jsonb_build_object('user_id', OLD.user_id, 'target_user_urn', OLD.target_user_urn);
    IF OLD.wanted_v > coalesce(pv, 0) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('user_followings', jsonb_build_object('user_id', OLD.user_id, 'target_user_urn', OLD.target_user_urn)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_user_followings'::regclass);
        RETURN NULL;
    END IF;
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_purge_guard BEFORE DELETE ON public.user_followings FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_purge_guard_user_followings();
ALTER TABLE public.user_followings ENABLE REPLICA TRIGGER zz_purge_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_user_followings()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF public.scd_role() = 'api' OR current_setting('scd.writer', true) = 'api' THEN
            NEW.wanted_v := public.scd_hlc(greatest(NEW.wanted_v, (SELECT max(x.purged_v) FROM public.repl_purges x WHERE x.tbl = 'user_followings' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'target_user_urn', NEW.target_user_urn))));
        END IF;
        RETURN NEW;
    END IF;
    IF (NEW.wanted_state, NEW.created_at) IS DISTINCT FROM (OLD.wanted_state, OLD.created_at) THEN
        NEW.wanted_v := public.scd_hlc(greatest(OLD.wanted_v, NEW.wanted_v, (SELECT max(x.purged_v) FROM public.repl_purges x WHERE x.tbl = 'user_followings' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'target_user_urn', NEW.target_user_urn))));
    ELSE
        NEW.wanted_v := greatest(OLD.wanted_v, NEW.wanted_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.user_followings FOR EACH ROW EXECUTE FUNCTION public.scd_ver_user_followings();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_followings FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_followings ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_merge_user_like_ranks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.like_created_at := OLD.like_created_at;
            NEW.rank := OLD.rank;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.user_like_ranks t WHERE (t.user_id, t.sc_track_id) = (NEW.user_id, NEW.sc_track_id);
        IF FOUND THEN
            IF NEW.row_v > o.row_v THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.user_like_ranks t SET
                    like_created_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.like_created_at ELSE o.like_created_at END,
                    rank = CASE WHEN NEW.row_v > o.row_v THEN NEW.rank ELSE o.rank END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE (t.user_id, t.sc_track_id) = (o.user_id, o.sc_track_id);
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'user_like_ranks' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id));
        IF tv IS NOT NULL AND NEW.row_v <= tv THEN
            INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'user_like_ranks', 'pk', jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id), 'v', tv)) ON CONFLICT DO NOTHING;
            PERFORM nextval('public.scd_resend_user_like_ranks'::regclass);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.user_like_ranks (like_created_at, rank, row_v, sc_track_id, user_id)
        VALUES (NEW.like_created_at, NEW.rank, NEW.row_v, NEW.sc_track_id, NEW.user_id)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.user_like_ranks still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.user_like_ranks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_user_like_ranks();
ALTER TABLE public.user_like_ranks ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_user_like_ranks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('user_like_ranks', jsonb_build_object('user_id', OLD.user_id, 'sc_track_id', OLD.sc_track_id), OLD.row_v, now());
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb BEFORE DELETE ON public.user_like_ranks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_user_like_ranks();
CREATE OR REPLACE FUNCTION public.scd_erase_user_like_ranks(k jsonb, upto bigint)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    DELETE FROM public.user_like_ranks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text AND t.row_v <= upto;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
    IF EXISTS (SELECT 1 FROM public.user_like_ranks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('user_like_ranks', k) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_user_like_ranks'::regclass);
    END IF;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_tomb_check_user_like_ranks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'user_like_ranks' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id));
    IF tv IS NOT NULL AND NEW.row_v <= tv THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'user_like_ranks', 'pk', jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id), 'v', tv)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_user_like_ranks'::regclass);
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.user_like_ranks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_user_like_ranks();
ALTER TABLE public.user_like_ranks ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_tomb_guard_user_like_ranks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN OLD;
    END IF;
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'user_like_ranks' AND x.pk = jsonb_build_object('user_id', OLD.user_id, 'sc_track_id', OLD.sc_track_id));
    IF tv IS NULL OR OLD.row_v <= tv THEN
        RETURN OLD;
    END IF;
    INSERT INTO public.repl_resend (tbl, pk) VALUES ('user_like_ranks', jsonb_build_object('user_id', OLD.user_id, 'sc_track_id', OLD.sc_track_id)) ON CONFLICT DO NOTHING;
    PERFORM nextval('public.scd_resend_user_like_ranks'::regclass);
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_guard BEFORE DELETE ON public.user_like_ranks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_guard_user_like_ranks();
ALTER TABLE public.user_like_ranks ENABLE REPLICA TRIGGER zz_tomb_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_user_like_ranks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(greatest(NEW.row_v, (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'user_like_ranks' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id))));
        RETURN NEW;
    END IF;
    IF (NEW.like_created_at, NEW.rank) IS DISTINCT FROM (OLD.like_created_at, OLD.rank) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.user_like_ranks FOR EACH ROW EXECUTE FUNCTION public.scd_ver_user_like_ranks();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_like_ranks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_like_ranks ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

DROP FUNCTION IF EXISTS public.scd_purge_intent_user_likes_playlists(jsonb);
CREATE OR REPLACE FUNCTION public.scd_purge_intent_user_likes_playlists(k jsonb, seen bigint)
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
    IF NOT FOUND OR v IS DISTINCT FROM seen THEN
        RETURN false;
    END IF;
    INSERT INTO public.repl_purges (tbl, pk, purged_v) VALUES ('user_likes_playlists', key, v);
    DELETE FROM public.user_likes_playlists t WHERE t.user_id = (k ->> 'user_id')::text AND t.playlist_urn = (k ->> 'playlist_urn')::text;
    RETURN true;
END
$f$;

CREATE OR REPLACE TRIGGER zz_guard_intent BEFORE INSERT OR UPDATE OF created_at, wanted_state, wanted_v ON public.user_likes_playlists FOR EACH STATEMENT WHEN (public.scd_role() = 'core') EXECUTE FUNCTION public.scd_guard('public.scd_guard_intent_user_likes_playlists', 'mirror', 'migration', 'repl', 'api');

CREATE OR REPLACE TRIGGER zz_guard_result BEFORE UPDATE OF progress, synced_at, synced_for_v ON public.user_likes_playlists FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_result_user_likes_playlists', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_merge_user_likes_playlists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
    pv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    pv := (SELECT max(x.purged_v) FROM public.repl_purges x WHERE x.tbl = 'user_likes_playlists' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'playlist_urn', NEW.playlist_urn));
    IF pv IS NOT NULL AND NEW.wanted_v <= pv THEN
        PERFORM nextval('public.scd_purge_block_user_likes_playlists'::regclass);
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.wanted_v < OLD.wanted_v OR (core AND NEW.wanted_v = OLD.wanted_v) THEN
            NEW.wanted_state := OLD.wanted_state;
            NEW.created_at := OLD.created_at;
            NEW.wanted_v := OLD.wanted_v;
        END IF;
        IF core THEN
            NEW.progress := OLD.progress;
            NEW.synced_at := OLD.synced_at;
            NEW.synced_for_v := OLD.synced_for_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.user_likes_playlists t WHERE (t.user_id, t.playlist_urn) = (NEW.user_id, NEW.playlist_urn);
        IF FOUND THEN
            IF NEW.wanted_v > o.wanted_v OR (((NEW.wanted_v & 15) <> public.scd_node_ord() OR NEW.wanted_v = o.wanted_v) AND NEW.created_at < o.created_at) OR (NOT core AND (NEW.progress, NEW.synced_at, NEW.synced_for_v) IS DISTINCT FROM (o.progress, o.synced_at, o.synced_for_v)) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.user_likes_playlists t SET
                    wanted_state = CASE WHEN NEW.wanted_v > o.wanted_v THEN NEW.wanted_state ELSE o.wanted_state END,
                    created_at = CASE WHEN ((NEW.wanted_v & 15) <> public.scd_node_ord() OR NEW.wanted_v = o.wanted_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.wanted_v > o.wanted_v THEN NEW.created_at ELSE o.created_at END,
                    wanted_v = greatest(o.wanted_v, NEW.wanted_v),
                    progress = CASE WHEN core THEN o.progress ELSE NEW.progress END,
                    synced_at = CASE WHEN core THEN o.synced_at ELSE NEW.synced_at END,
                    synced_for_v = CASE WHEN core THEN o.synced_for_v ELSE NEW.synced_for_v END
                WHERE (t.user_id, t.playlist_urn) = (o.user_id, o.playlist_urn);
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.user_likes_playlists (created_at, last_read_at, playlist_urn, progress, synced_at, synced_for_v, user_id, wanted_state, wanted_v)
        VALUES (NEW.created_at, NEW.last_read_at, NEW.playlist_urn, NEW.progress, NEW.synced_at, NEW.synced_for_v, NEW.user_id, NEW.wanted_state, NEW.wanted_v)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.user_likes_playlists still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.user_likes_playlists FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_user_likes_playlists();
ALTER TABLE public.user_likes_playlists ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_purge_guard_user_likes_playlists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    pv bigint;
BEGIN
    SELECT max(x.purged_v) INTO pv FROM public.repl_purges x WHERE x.tbl = 'user_likes_playlists' AND x.pk = jsonb_build_object('user_id', OLD.user_id, 'playlist_urn', OLD.playlist_urn);
    IF OLD.wanted_v > coalesce(pv, 0) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('user_likes_playlists', jsonb_build_object('user_id', OLD.user_id, 'playlist_urn', OLD.playlist_urn)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_user_likes_playlists'::regclass);
        RETURN NULL;
    END IF;
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_purge_guard BEFORE DELETE ON public.user_likes_playlists FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_purge_guard_user_likes_playlists();
ALTER TABLE public.user_likes_playlists ENABLE REPLICA TRIGGER zz_purge_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_user_likes_playlists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF public.scd_role() = 'api' OR current_setting('scd.writer', true) = 'api' THEN
            NEW.wanted_v := public.scd_hlc(greatest(NEW.wanted_v, (SELECT max(x.purged_v) FROM public.repl_purges x WHERE x.tbl = 'user_likes_playlists' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'playlist_urn', NEW.playlist_urn))));
        END IF;
        RETURN NEW;
    END IF;
    IF (NEW.wanted_state, NEW.created_at) IS DISTINCT FROM (OLD.wanted_state, OLD.created_at) THEN
        NEW.wanted_v := public.scd_hlc(greatest(OLD.wanted_v, NEW.wanted_v, (SELECT max(x.purged_v) FROM public.repl_purges x WHERE x.tbl = 'user_likes_playlists' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'playlist_urn', NEW.playlist_urn))));
    ELSE
        NEW.wanted_v := greatest(OLD.wanted_v, NEW.wanted_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.user_likes_playlists FOR EACH ROW EXECUTE FUNCTION public.scd_ver_user_likes_playlists();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_likes_playlists FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_likes_playlists ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

DROP FUNCTION IF EXISTS public.scd_purge_intent_user_likes_tracks(jsonb);
CREATE OR REPLACE FUNCTION public.scd_purge_intent_user_likes_tracks(k jsonb, seen bigint)
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
    IF NOT FOUND OR v IS DISTINCT FROM seen THEN
        RETURN false;
    END IF;
    INSERT INTO public.repl_purges (tbl, pk, purged_v) VALUES ('user_likes_tracks', key, v);
    DELETE FROM public.user_likes_tracks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text;
    RETURN true;
END
$f$;

CREATE OR REPLACE TRIGGER zz_guard_intent BEFORE INSERT OR UPDATE OF created_at, wanted_state, wanted_v ON public.user_likes_tracks FOR EACH STATEMENT WHEN (public.scd_role() = 'core') EXECUTE FUNCTION public.scd_guard('public.scd_guard_intent_user_likes_tracks', 'mirror', 'migration', 'repl', 'api');

CREATE OR REPLACE TRIGGER zz_guard_result BEFORE UPDATE OF liked_at, progress, synced_at, synced_for_v ON public.user_likes_tracks FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_result_user_likes_tracks', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_merge_user_likes_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
    pv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    pv := (SELECT max(x.purged_v) FROM public.repl_purges x WHERE x.tbl = 'user_likes_tracks' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id));
    IF pv IS NOT NULL AND NEW.wanted_v <= pv THEN
        PERFORM nextval('public.scd_purge_block_user_likes_tracks'::regclass);
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.wanted_v < OLD.wanted_v OR (core AND NEW.wanted_v = OLD.wanted_v) THEN
            NEW.wanted_state := OLD.wanted_state;
            NEW.created_at := OLD.created_at;
            NEW.wanted_v := OLD.wanted_v;
        END IF;
        IF core THEN
            NEW.progress := OLD.progress;
            NEW.synced_at := OLD.synced_at;
            NEW.liked_at := OLD.liked_at;
            NEW.synced_for_v := OLD.synced_for_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.user_likes_tracks t WHERE (t.user_id, t.sc_track_id) = (NEW.user_id, NEW.sc_track_id);
        IF FOUND THEN
            IF NEW.wanted_v > o.wanted_v OR (((NEW.wanted_v & 15) <> public.scd_node_ord() OR NEW.wanted_v = o.wanted_v) AND NEW.created_at < o.created_at) OR (NOT core AND (NEW.progress, NEW.synced_at, NEW.liked_at, NEW.synced_for_v) IS DISTINCT FROM (o.progress, o.synced_at, o.liked_at, o.synced_for_v)) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.user_likes_tracks t SET
                    wanted_state = CASE WHEN NEW.wanted_v > o.wanted_v THEN NEW.wanted_state ELSE o.wanted_state END,
                    created_at = CASE WHEN ((NEW.wanted_v & 15) <> public.scd_node_ord() OR NEW.wanted_v = o.wanted_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.wanted_v > o.wanted_v THEN NEW.created_at ELSE o.created_at END,
                    wanted_v = greatest(o.wanted_v, NEW.wanted_v),
                    progress = CASE WHEN core THEN o.progress ELSE NEW.progress END,
                    synced_at = CASE WHEN core THEN o.synced_at ELSE NEW.synced_at END,
                    liked_at = CASE WHEN core THEN o.liked_at ELSE NEW.liked_at END,
                    synced_for_v = CASE WHEN core THEN o.synced_for_v ELSE NEW.synced_for_v END
                WHERE (t.user_id, t.sc_track_id) = (o.user_id, o.sc_track_id);
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.user_likes_tracks (created_at, last_read_at, liked_at, progress, sc_track_id, synced_at, synced_for_v, user_id, wanted_state, wanted_v)
        VALUES (NEW.created_at, NEW.last_read_at, NEW.liked_at, NEW.progress, NEW.sc_track_id, NEW.synced_at, NEW.synced_for_v, NEW.user_id, NEW.wanted_state, NEW.wanted_v)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.user_likes_tracks still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.user_likes_tracks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_user_likes_tracks();
ALTER TABLE public.user_likes_tracks ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_purge_guard_user_likes_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    pv bigint;
BEGIN
    SELECT max(x.purged_v) INTO pv FROM public.repl_purges x WHERE x.tbl = 'user_likes_tracks' AND x.pk = jsonb_build_object('user_id', OLD.user_id, 'sc_track_id', OLD.sc_track_id);
    IF OLD.wanted_v > coalesce(pv, 0) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('user_likes_tracks', jsonb_build_object('user_id', OLD.user_id, 'sc_track_id', OLD.sc_track_id)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_user_likes_tracks'::regclass);
        RETURN NULL;
    END IF;
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_purge_guard BEFORE DELETE ON public.user_likes_tracks FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_purge_guard_user_likes_tracks();
ALTER TABLE public.user_likes_tracks ENABLE REPLICA TRIGGER zz_purge_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_user_likes_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF public.scd_role() = 'api' OR current_setting('scd.writer', true) = 'api' THEN
            NEW.wanted_v := public.scd_hlc(greatest(NEW.wanted_v, (SELECT max(x.purged_v) FROM public.repl_purges x WHERE x.tbl = 'user_likes_tracks' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id))));
        END IF;
        RETURN NEW;
    END IF;
    IF (NEW.wanted_state, NEW.created_at) IS DISTINCT FROM (OLD.wanted_state, OLD.created_at) THEN
        NEW.wanted_v := public.scd_hlc(greatest(OLD.wanted_v, NEW.wanted_v, (SELECT max(x.purged_v) FROM public.repl_purges x WHERE x.tbl = 'user_likes_tracks' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id))));
    ELSE
        NEW.wanted_v := greatest(OLD.wanted_v, NEW.wanted_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.user_likes_tracks FOR EACH ROW EXECUTE FUNCTION public.scd_ver_user_likes_tracks();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_likes_tracks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_likes_tracks ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.user_owned_playlists FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_user_owned_playlists', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_user_owned_playlists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_owned_playlists' AND p.pk = jsonb_build_object('user_id', OLD.user_id, 'playlist_urn', OLD.playlist_urn)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.user_id, NEW.playlist_urn) IS DISTINCT FROM (OLD.user_id, OLD.playlist_urn) THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_owned_playlists' AND p.pk = jsonb_build_object('user_id', OLD.user_id, 'playlist_urn', OLD.playlist_urn)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_owned_playlists' AND p.pk = jsonb_build_object('user_id', NEW.user_id, 'playlist_urn', NEW.playlist_urn)) THEN
                    INSERT INTO public.user_owned_playlists (created_at, last_read_at, playlist_urn, progress, synced_at, user_id) VALUES (NEW.created_at, NEW.last_read_at, NEW.playlist_urn, NEW.progress, NEW.synced_at, NEW.user_id);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_owned_playlists' AND p.pk = jsonb_build_object('user_id', NEW.user_id, 'playlist_urn', NEW.playlist_urn)) THEN
                DELETE FROM public.user_owned_playlists t WHERE (t.user_id, t.playlist_urn) = (OLD.user_id, OLD.playlist_urn);
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_owned_playlists' AND p.pk = jsonb_build_object('user_id', NEW.user_id, 'playlist_urn', NEW.playlist_urn)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.user_owned_playlists t WHERE (t.user_id, t.playlist_urn) = (NEW.user_id, NEW.playlist_urn) FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_owned_playlists' AND p.pk = jsonb_build_object('user_id', NEW.user_id, 'playlist_urn', NEW.playlist_urn)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.user_owned_playlists t SET created_at = NEW.created_at, progress = NEW.progress, synced_at = NEW.synced_at WHERE (t.user_id, t.playlist_urn) = (NEW.user_id, NEW.playlist_urn);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.user_owned_playlists (created_at, last_read_at, playlist_urn, progress, synced_at, user_id)
        VALUES (NEW.created_at, NEW.last_read_at, NEW.playlist_urn, NEW.progress, NEW.synced_at, NEW.user_id)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.user_owned_playlists still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.user_owned_playlists FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_user_owned_playlists();
ALTER TABLE public.user_owned_playlists ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_owned_playlists FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_owned_playlists ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.user_owned_tracks FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_user_owned_tracks', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_user_owned_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_owned_tracks' AND p.pk = jsonb_build_object('user_id', OLD.user_id, 'sc_track_id', OLD.sc_track_id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.user_id, NEW.sc_track_id) IS DISTINCT FROM (OLD.user_id, OLD.sc_track_id) THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_owned_tracks' AND p.pk = jsonb_build_object('user_id', OLD.user_id, 'sc_track_id', OLD.sc_track_id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_owned_tracks' AND p.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id)) THEN
                    INSERT INTO public.user_owned_tracks (created_at, last_read_at, progress, sc_track_id, synced_at, user_id) VALUES (NEW.created_at, NEW.last_read_at, NEW.progress, NEW.sc_track_id, NEW.synced_at, NEW.user_id);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_owned_tracks' AND p.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id)) THEN
                DELETE FROM public.user_owned_tracks t WHERE (t.user_id, t.sc_track_id) = (OLD.user_id, OLD.sc_track_id);
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_owned_tracks' AND p.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.user_owned_tracks t WHERE (t.user_id, t.sc_track_id) = (NEW.user_id, NEW.sc_track_id) FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_owned_tracks' AND p.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.user_owned_tracks t SET created_at = NEW.created_at, progress = NEW.progress, synced_at = NEW.synced_at WHERE (t.user_id, t.sc_track_id) = (NEW.user_id, NEW.sc_track_id);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.user_owned_tracks (created_at, last_read_at, progress, sc_track_id, synced_at, user_id)
        VALUES (NEW.created_at, NEW.last_read_at, NEW.progress, NEW.sc_track_id, NEW.synced_at, NEW.user_id)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.user_owned_tracks still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.user_owned_tracks FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_user_owned_tracks();
ALTER TABLE public.user_owned_tracks ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_owned_tracks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_owned_tracks ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.user_profiles FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_user_profiles', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_user_profiles()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_profiles' AND p.pk = jsonb_build_object('soundcloud_user_id', OLD.soundcloud_user_id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.soundcloud_user_id IS DISTINCT FROM OLD.soundcloud_user_id THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_profiles' AND p.pk = jsonb_build_object('soundcloud_user_id', OLD.soundcloud_user_id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_profiles' AND p.pk = jsonb_build_object('soundcloud_user_id', NEW.soundcloud_user_id)) THEN
                    INSERT INTO public.user_profiles (profile_json, sc_observation, soundcloud_user_id, synced_at) VALUES (NEW.profile_json, NEW.sc_observation, NEW.soundcloud_user_id, NEW.synced_at);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_profiles' AND p.pk = jsonb_build_object('soundcloud_user_id', NEW.soundcloud_user_id)) THEN
                DELETE FROM public.user_profiles t WHERE t.soundcloud_user_id = OLD.soundcloud_user_id;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_profiles' AND p.pk = jsonb_build_object('soundcloud_user_id', NEW.soundcloud_user_id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.user_profiles t WHERE t.soundcloud_user_id = NEW.soundcloud_user_id FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_profiles' AND p.pk = jsonb_build_object('soundcloud_user_id', NEW.soundcloud_user_id)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.user_profiles t SET profile_json = NEW.profile_json, sc_observation = NEW.sc_observation, synced_at = NEW.synced_at WHERE t.soundcloud_user_id = NEW.soundcloud_user_id;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.user_profiles (profile_json, sc_observation, soundcloud_user_id, synced_at)
        VALUES (NEW.profile_json, NEW.sc_observation, NEW.soundcloud_user_id, NEW.synced_at)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.user_profiles still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.user_profiles FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_user_profiles();
ALTER TABLE public.user_profiles ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_profiles FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_profiles ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_merge_user_superlike_ranks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.rank := OLD.rank;
            NEW.superlike_created_at := OLD.superlike_created_at;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.user_superlike_ranks t WHERE (t.user_id, t.sc_track_id) = (NEW.user_id, NEW.sc_track_id);
        IF FOUND THEN
            IF NEW.row_v > o.row_v THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.user_superlike_ranks t SET
                    rank = CASE WHEN NEW.row_v > o.row_v THEN NEW.rank ELSE o.rank END,
                    superlike_created_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.superlike_created_at ELSE o.superlike_created_at END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE (t.user_id, t.sc_track_id) = (o.user_id, o.sc_track_id);
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'user_superlike_ranks' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id));
        IF tv IS NOT NULL AND NEW.row_v <= tv THEN
            INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'user_superlike_ranks', 'pk', jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id), 'v', tv)) ON CONFLICT DO NOTHING;
            PERFORM nextval('public.scd_resend_user_superlike_ranks'::regclass);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.user_superlike_ranks (rank, row_v, sc_track_id, superlike_created_at, user_id)
        VALUES (NEW.rank, NEW.row_v, NEW.sc_track_id, NEW.superlike_created_at, NEW.user_id)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.user_superlike_ranks still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.user_superlike_ranks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_user_superlike_ranks();
ALTER TABLE public.user_superlike_ranks ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_user_superlike_ranks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('user_superlike_ranks', jsonb_build_object('user_id', OLD.user_id, 'sc_track_id', OLD.sc_track_id), OLD.row_v, now());
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb BEFORE DELETE ON public.user_superlike_ranks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_user_superlike_ranks();
CREATE OR REPLACE FUNCTION public.scd_erase_user_superlike_ranks(k jsonb, upto bigint)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    DELETE FROM public.user_superlike_ranks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text AND t.row_v <= upto;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
    IF EXISTS (SELECT 1 FROM public.user_superlike_ranks t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('user_superlike_ranks', k) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_user_superlike_ranks'::regclass);
    END IF;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_tomb_check_user_superlike_ranks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'user_superlike_ranks' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id));
    IF tv IS NOT NULL AND NEW.row_v <= tv THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'user_superlike_ranks', 'pk', jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id), 'v', tv)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_user_superlike_ranks'::regclass);
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.user_superlike_ranks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_user_superlike_ranks();
ALTER TABLE public.user_superlike_ranks ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_tomb_guard_user_superlike_ranks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN OLD;
    END IF;
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'user_superlike_ranks' AND x.pk = jsonb_build_object('user_id', OLD.user_id, 'sc_track_id', OLD.sc_track_id));
    IF tv IS NULL OR OLD.row_v <= tv THEN
        RETURN OLD;
    END IF;
    INSERT INTO public.repl_resend (tbl, pk) VALUES ('user_superlike_ranks', jsonb_build_object('user_id', OLD.user_id, 'sc_track_id', OLD.sc_track_id)) ON CONFLICT DO NOTHING;
    PERFORM nextval('public.scd_resend_user_superlike_ranks'::regclass);
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_guard BEFORE DELETE ON public.user_superlike_ranks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_guard_user_superlike_ranks();
ALTER TABLE public.user_superlike_ranks ENABLE REPLICA TRIGGER zz_tomb_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_user_superlike_ranks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(greatest(NEW.row_v, (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'user_superlike_ranks' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id))));
        RETURN NEW;
    END IF;
    IF (NEW.rank, NEW.superlike_created_at) IS DISTINCT FROM (OLD.rank, OLD.superlike_created_at) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.user_superlike_ranks FOR EACH ROW EXECUTE FUNCTION public.scd_ver_user_superlike_ranks();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_superlike_ranks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_superlike_ranks ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE FUNCTION public.scd_merge_user_superlikes()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
    core boolean := public.scd_role() = 'core';
    n bigint;
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v < OLD.row_v OR (core AND NEW.row_v = OLD.row_v) THEN
            NEW.created_at := OLD.created_at;
            NEW.like_created_at := OLD.like_created_at;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        SELECT t.* INTO o FROM public.user_superlikes t WHERE (t.user_id, t.sc_track_id) = (NEW.user_id, NEW.sc_track_id);
        IF FOUND THEN
            IF NEW.row_v > o.row_v OR (((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) AND NEW.created_at < o.created_at) THEN
                PERFORM pg_catalog.set_config('scd.merging', 'on', true);
                UPDATE public.user_superlikes t SET
                    created_at = CASE WHEN ((NEW.row_v & 15) <> public.scd_node_ord() OR NEW.row_v = o.row_v) THEN least(o.created_at, NEW.created_at) WHEN NEW.row_v > o.row_v THEN NEW.created_at ELSE o.created_at END,
                    like_created_at = CASE WHEN NEW.row_v > o.row_v THEN NEW.like_created_at ELSE o.like_created_at END,
                    row_v = greatest(o.row_v, NEW.row_v)
                WHERE (t.user_id, t.sc_track_id) = (o.user_id, o.sc_track_id);
                GET DIAGNOSTICS n = ROW_COUNT;
                PERFORM pg_catalog.set_config('scd.merging', '', true);
                IF n = 0 THEN
                    CONTINUE;
                END IF;
            END IF;
            RETURN NULL;
        END IF;
        tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'user_superlikes' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id));
        IF tv IS NOT NULL AND NEW.row_v <= tv THEN
            INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'user_superlikes', 'pk', jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id), 'v', tv)) ON CONFLICT DO NOTHING;
            PERFORM nextval('public.scd_resend_user_superlikes'::regclass);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.user_superlikes (created_at, like_created_at, row_v, sc_track_id, user_id)
        VALUES (NEW.created_at, NEW.like_created_at, NEW.row_v, NEW.sc_track_id, NEW.user_id)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.user_superlikes still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.user_superlikes FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_user_superlikes();
ALTER TABLE public.user_superlikes ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_user_superlikes()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('user_superlikes', jsonb_build_object('user_id', OLD.user_id, 'sc_track_id', OLD.sc_track_id), OLD.row_v, now());
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb BEFORE DELETE ON public.user_superlikes FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_user_superlikes();
CREATE OR REPLACE FUNCTION public.scd_erase_user_superlikes(k jsonb, upto bigint)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM pg_catalog.set_config('scd.merging', 'on', true);
    DELETE FROM public.user_superlikes t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text AND t.row_v <= upto;
    PERFORM pg_catalog.set_config('scd.merging', '', true);
    IF EXISTS (SELECT 1 FROM public.user_superlikes t WHERE t.user_id = (k ->> 'user_id')::text AND t.sc_track_id = (k ->> 'sc_track_id')::text) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('user_superlikes', k) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_user_superlikes'::regclass);
    END IF;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_tomb_check_user_superlikes()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'user_superlikes' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id));
    IF tv IS NOT NULL AND NEW.row_v <= tv THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('repl_tombstones', jsonb_build_object('tbl', 'user_superlikes', 'pk', jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id), 'v', tv)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_user_superlikes'::regclass);
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.user_superlikes FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_user_superlikes();
ALTER TABLE public.user_superlikes ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_tomb_guard_user_superlikes()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        RETURN OLD;
    END IF;
    tv := (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'user_superlikes' AND x.pk = jsonb_build_object('user_id', OLD.user_id, 'sc_track_id', OLD.sc_track_id));
    IF tv IS NULL OR OLD.row_v <= tv THEN
        RETURN OLD;
    END IF;
    INSERT INTO public.repl_resend (tbl, pk) VALUES ('user_superlikes', jsonb_build_object('user_id', OLD.user_id, 'sc_track_id', OLD.sc_track_id)) ON CONFLICT DO NOTHING;
    PERFORM nextval('public.scd_resend_user_superlikes'::regclass);
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_guard BEFORE DELETE ON public.user_superlikes FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_guard_user_superlikes();
ALTER TABLE public.user_superlikes ENABLE REPLICA TRIGGER zz_tomb_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_user_superlikes()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(greatest(NEW.row_v, (SELECT max(x.v) FROM public.repl_tombstones x WHERE x.tbl = 'user_superlikes' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id))));
        RETURN NEW;
    END IF;
    IF (NEW.created_at, NEW.like_created_at) IS DISTINCT FROM (OLD.created_at, OLD.like_created_at) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.user_superlikes FOR EACH ROW EXECUTE FUNCTION public.scd_ver_user_superlikes();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_superlikes FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_superlikes ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.user_taste_vectors FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_user_taste_vectors', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_user_taste_vectors()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_taste_vectors' AND p.pk = jsonb_build_object('sc_user_id', OLD.sc_user_id, 'version', OLD.version)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.sc_user_id, NEW.version) IS DISTINCT FROM (OLD.sc_user_id, OLD.version) THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_taste_vectors' AND p.pk = jsonb_build_object('sc_user_id', OLD.sc_user_id, 'version', OLD.version)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_taste_vectors' AND p.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id, 'version', NEW.version)) THEN
                    INSERT INTO public.user_taste_vectors (sc_user_id, updated_at, vec, version) VALUES (NEW.sc_user_id, NEW.updated_at, NEW.vec, NEW.version);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_taste_vectors' AND p.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id, 'version', NEW.version)) THEN
                DELETE FROM public.user_taste_vectors t WHERE (t.sc_user_id, t.version) = (OLD.sc_user_id, OLD.version);
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_taste_vectors' AND p.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id, 'version', NEW.version)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.user_taste_vectors t WHERE (t.sc_user_id, t.version) = (NEW.sc_user_id, NEW.version) FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_taste_vectors' AND p.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id, 'version', NEW.version)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.user_taste_vectors t SET updated_at = NEW.updated_at, vec = NEW.vec WHERE (t.sc_user_id, t.version) = (NEW.sc_user_id, NEW.version);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.user_taste_vectors (sc_user_id, updated_at, vec, version)
        VALUES (NEW.sc_user_id, NEW.updated_at, NEW.vec, NEW.version)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.user_taste_vectors still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.user_taste_vectors FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_user_taste_vectors();
ALTER TABLE public.user_taste_vectors ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_taste_vectors FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_taste_vectors ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.user_web_profiles FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_user_web_profiles', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_user_web_profiles()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_web_profiles' AND p.pk = jsonb_build_object('sc_user_id', OLD.sc_user_id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.sc_user_id IS DISTINCT FROM OLD.sc_user_id THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_web_profiles' AND p.pk = jsonb_build_object('sc_user_id', OLD.sc_user_id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_web_profiles' AND p.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id)) THEN
                    INSERT INTO public.user_web_profiles (profiles, sc_observation, sc_user_id, synced_at) VALUES (NEW.profiles, NEW.sc_observation, NEW.sc_user_id, NEW.synced_at);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_web_profiles' AND p.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id)) THEN
                DELETE FROM public.user_web_profiles t WHERE t.sc_user_id = OLD.sc_user_id;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_web_profiles' AND p.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.user_web_profiles t WHERE t.sc_user_id = NEW.sc_user_id FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_web_profiles' AND p.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.user_web_profiles t SET profiles = NEW.profiles, sc_observation = NEW.sc_observation, synced_at = NEW.synced_at WHERE t.sc_user_id = NEW.sc_user_id;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.user_web_profiles (profiles, sc_observation, sc_user_id, synced_at)
        VALUES (NEW.profiles, NEW.sc_observation, NEW.sc_user_id, NEW.synced_at)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.user_web_profiles still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.user_web_profiles FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_user_web_profiles();
ALTER TABLE public.user_web_profiles ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_web_profiles FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_web_profiles ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.users FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_users', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_users()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
    k0 boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'users' AND p.pk = jsonb_build_object('sc_user_id', OLD.sc_user_id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.sc_user_id IS DISTINCT FROM OLD.sc_user_id THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'users' AND p.pk = jsonb_build_object('sc_user_id', OLD.sc_user_id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'users' AND p.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id)) THEN
                    INSERT INTO public.users (avatar_url, city, comments_count, country, created_at, description, first_name, followers_count, followings_count, full_name, kind, last_name, last_read_at, permalink, permalink_url, playlists_count, reposts_count, sc_created_at, sc_last_modified, sc_observation, sc_synced_at, sc_user_id, tracks_count, updated_at, urn, username, username_normalized, verified) VALUES (NEW.avatar_url, NEW.city, NEW.comments_count, NEW.country, NEW.created_at, NEW.description, NEW.first_name, NEW.followers_count, NEW.followings_count, NEW.full_name, NEW.kind, NEW.last_name, NEW.last_read_at, NEW.permalink, NEW.permalink_url, NEW.playlists_count, NEW.reposts_count, NEW.sc_created_at, NEW.sc_last_modified, NEW.sc_observation, NEW.sc_synced_at, NEW.sc_user_id, NEW.tracks_count, NEW.updated_at, NEW.urn, NEW.username, NEW.username_normalized, NEW.verified);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'users' AND p.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id)) THEN
                DELETE FROM public.users t WHERE t.sc_user_id = OLD.sc_user_id;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'users' AND p.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        k0 := NEW.urn IS DISTINCT FROM OLD.urn;
        IF k0 THEN
            FOR r IN SELECT t.sc_user_id FROM public.users t WHERE t.urn = NEW.urn AND t.sc_user_id IS DISTINCT FROM NEW.sc_user_id LOOP
                PERFORM public.scd_purge_users(jsonb_build_object('sc_user_id', r.sc_user_id));
            END LOOP;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.users t WHERE t.sc_user_id = NEW.sc_user_id FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'users' AND p.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id)) THEN
            RETURN NULL;
        END IF;
        FOR r IN SELECT t.sc_user_id FROM public.users t WHERE t.urn = NEW.urn AND t.sc_user_id IS DISTINCT FROM NEW.sc_user_id LOOP
            PERFORM public.scd_purge_users(jsonb_build_object('sc_user_id', r.sc_user_id));
        END LOOP;
        IF held THEN
            UPDATE public.users t SET avatar_url = NEW.avatar_url, city = NEW.city, comments_count = NEW.comments_count, country = NEW.country, created_at = NEW.created_at, description = NEW.description, first_name = NEW.first_name, followers_count = NEW.followers_count, followings_count = NEW.followings_count, full_name = NEW.full_name, kind = NEW.kind, last_name = NEW.last_name, permalink = NEW.permalink, permalink_url = NEW.permalink_url, playlists_count = NEW.playlists_count, reposts_count = NEW.reposts_count, sc_created_at = NEW.sc_created_at, sc_last_modified = NEW.sc_last_modified, sc_observation = NEW.sc_observation, sc_synced_at = NEW.sc_synced_at, tracks_count = NEW.tracks_count, updated_at = NEW.updated_at, urn = NEW.urn, username = NEW.username, username_normalized = NEW.username_normalized, verified = NEW.verified WHERE t.sc_user_id = NEW.sc_user_id;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.users (avatar_url, city, comments_count, country, created_at, description, first_name, followers_count, followings_count, full_name, kind, last_name, last_read_at, permalink, permalink_url, playlists_count, reposts_count, sc_created_at, sc_last_modified, sc_observation, sc_synced_at, sc_user_id, tracks_count, updated_at, urn, username, username_normalized, verified)
        VALUES (NEW.avatar_url, NEW.city, NEW.comments_count, NEW.country, NEW.created_at, NEW.description, NEW.first_name, NEW.followers_count, NEW.followings_count, NEW.full_name, NEW.kind, NEW.last_name, NEW.last_read_at, NEW.permalink, NEW.permalink_url, NEW.playlists_count, NEW.reposts_count, NEW.sc_created_at, NEW.sc_last_modified, NEW.sc_observation, NEW.sc_synced_at, NEW.sc_user_id, NEW.tracks_count, NEW.updated_at, NEW.urn, NEW.username, NEW.username_normalized, NEW.verified)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.users still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.users FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_users();
ALTER TABLE public.users ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.users FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.users ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.wanted_track_albums FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_wanted_track_albums', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_wanted_track_albums()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_track_albums' AND p.pk = jsonb_build_object('wanted_track_id', OLD.wanted_track_id, 'album_id', OLD.album_id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.wanted_track_id, NEW.album_id) IS DISTINCT FROM (OLD.wanted_track_id, OLD.album_id) THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_track_albums' AND p.pk = jsonb_build_object('wanted_track_id', OLD.wanted_track_id, 'album_id', OLD.album_id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_track_albums' AND p.pk = jsonb_build_object('wanted_track_id', NEW.wanted_track_id, 'album_id', NEW.album_id)) THEN
                    INSERT INTO public.wanted_track_albums (album_id, "position", wanted_track_id) VALUES (NEW.album_id, NEW."position", NEW.wanted_track_id);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_track_albums' AND p.pk = jsonb_build_object('wanted_track_id', NEW.wanted_track_id, 'album_id', NEW.album_id)) THEN
                DELETE FROM public.wanted_track_albums t WHERE (t.wanted_track_id, t.album_id) = (OLD.wanted_track_id, OLD.album_id);
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_track_albums' AND p.pk = jsonb_build_object('wanted_track_id', NEW.wanted_track_id, 'album_id', NEW.album_id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.wanted_track_albums t WHERE (t.wanted_track_id, t.album_id) = (NEW.wanted_track_id, NEW.album_id) FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_track_albums' AND p.pk = jsonb_build_object('wanted_track_id', NEW.wanted_track_id, 'album_id', NEW.album_id)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.wanted_track_albums t SET "position" = NEW."position" WHERE (t.wanted_track_id, t.album_id) = (NEW.wanted_track_id, NEW.album_id);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.wanted_track_albums (album_id, "position", wanted_track_id)
        VALUES (NEW.album_id, NEW."position", NEW.wanted_track_id)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.wanted_track_albums still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.wanted_track_albums FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_wanted_track_albums();
ALTER TABLE public.wanted_track_albums ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.wanted_track_albums FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.wanted_track_albums ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.wanted_track_artists FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_wanted_track_artists', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_wanted_track_artists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_track_artists' AND p.pk = jsonb_build_object('wanted_track_id', OLD.wanted_track_id, 'artist_id', OLD.artist_id, 'role', OLD.role)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.wanted_track_id, NEW.artist_id, NEW.role) IS DISTINCT FROM (OLD.wanted_track_id, OLD.artist_id, OLD.role) THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_track_artists' AND p.pk = jsonb_build_object('wanted_track_id', OLD.wanted_track_id, 'artist_id', OLD.artist_id, 'role', OLD.role)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_track_artists' AND p.pk = jsonb_build_object('wanted_track_id', NEW.wanted_track_id, 'artist_id', NEW.artist_id, 'role', NEW.role)) THEN
                    INSERT INTO public.wanted_track_artists (artist_id, "position", role, wanted_track_id) VALUES (NEW.artist_id, NEW."position", NEW.role, NEW.wanted_track_id);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_track_artists' AND p.pk = jsonb_build_object('wanted_track_id', NEW.wanted_track_id, 'artist_id', NEW.artist_id, 'role', NEW.role)) THEN
                DELETE FROM public.wanted_track_artists t WHERE (t.wanted_track_id, t.artist_id, t.role) = (OLD.wanted_track_id, OLD.artist_id, OLD.role);
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_track_artists' AND p.pk = jsonb_build_object('wanted_track_id', NEW.wanted_track_id, 'artist_id', NEW.artist_id, 'role', NEW.role)) THEN
                RETURN NULL;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.wanted_track_artists t WHERE (t.wanted_track_id, t.artist_id, t.role) = (NEW.wanted_track_id, NEW.artist_id, NEW.role) FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_track_artists' AND p.pk = jsonb_build_object('wanted_track_id', NEW.wanted_track_id, 'artist_id', NEW.artist_id, 'role', NEW.role)) THEN
            RETURN NULL;
        END IF;
        IF held THEN
            UPDATE public.wanted_track_artists t SET "position" = NEW."position" WHERE (t.wanted_track_id, t.artist_id, t.role) = (NEW.wanted_track_id, NEW.artist_id, NEW.role);
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.wanted_track_artists (artist_id, "position", role, wanted_track_id)
        VALUES (NEW.artist_id, NEW."position", NEW.role, NEW.wanted_track_id)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.wanted_track_artists still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.wanted_track_artists FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_wanted_track_artists();
ALTER TABLE public.wanted_track_artists ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.wanted_track_artists FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.wanted_track_artists ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.wanted_track_work_aliases FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_wanted_track_work_aliases', 'cmd', 'migration', 'repl');
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
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

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.wanted_tracks FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_wanted_tracks', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_wanted_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    n bigint;
    held boolean;
    k0 boolean;
    k1 boolean;
BEGIN
    IF current_setting('scd.merging', true) = 'on' THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' THEN
        IF pg_catalog.pg_trigger_depth() = 1 AND EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_tracks' AND p.pk = jsonb_build_object('id', OLD.id)) THEN
            RETURN NULL;
        END IF;
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.id IS DISTINCT FROM OLD.id THEN
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_tracks' AND p.pk = jsonb_build_object('id', OLD.id)) THEN
                IF NOT EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_tracks' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                    INSERT INTO public.wanted_tracks (discovered_at, duration_ms, external_id, fetched_at, id, isrc, normalized_title, primary_artist_id, recording_key, release_year, resolve_attempts, resolve_error, resolve_locked_at, resolve_next_run_at, source, status, title, track_id, updated_at, work_key, work_normalizer_version, work_reconciled_at) VALUES (NEW.discovered_at, NEW.duration_ms, NEW.external_id, NEW.fetched_at, NEW.id, NEW.isrc, NEW.normalized_title, NEW.primary_artist_id, NEW.recording_key, NEW.release_year, NEW.resolve_attempts, NEW.resolve_error, NEW.resolve_locked_at, NEW.resolve_next_run_at, NEW.source, NEW.status, NEW.title, NEW.track_id, NEW.updated_at, NEW.work_key, NEW.work_normalizer_version, NEW.work_reconciled_at);
                END IF;
                RETURN NULL;
            END IF;
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_tracks' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                DELETE FROM public.wanted_tracks t WHERE t.id = OLD.id;
                RETURN NULL;
            END IF;
        ELSE
            IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_tracks' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
                RETURN NULL;
            END IF;
        END IF;
        k0 := (NEW.source, NEW.external_id, ((NEW.external_id IS NOT NULL))) IS DISTINCT FROM (OLD.source, OLD.external_id, ((OLD.external_id IS NOT NULL)));
        k1 := (NEW.isrc, ((NEW.isrc IS NOT NULL))) IS DISTINCT FROM (OLD.isrc, ((OLD.isrc IS NOT NULL)));
        IF k0 AND ((NEW.external_id IS NOT NULL)) THEN
            FOR r IN SELECT t.id FROM public.wanted_tracks t WHERE (t.source, t.external_id) = (NEW.source, NEW.external_id) AND ((t.external_id IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_wanted_tracks(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        IF k1 AND ((NEW.isrc IS NOT NULL)) THEN
            FOR r IN SELECT t.id FROM public.wanted_tracks t WHERE t.isrc = NEW.isrc AND ((t.isrc IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_wanted_tracks(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        RETURN NEW;
    END IF;
    FOR attempt IN 1..3 LOOP
        PERFORM 1 FROM public.wanted_tracks t WHERE t.id = NEW.id FOR UPDATE;
        held := FOUND;
        IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_tracks' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
            RETURN NULL;
        END IF;
        IF ((NEW.external_id IS NOT NULL)) THEN
            FOR r IN SELECT t.id FROM public.wanted_tracks t WHERE (t.source, t.external_id) = (NEW.source, NEW.external_id) AND ((t.external_id IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_wanted_tracks(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        IF ((NEW.isrc IS NOT NULL)) THEN
            FOR r IN SELECT t.id FROM public.wanted_tracks t WHERE t.isrc = NEW.isrc AND ((t.isrc IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
                PERFORM public.scd_purge_wanted_tracks(jsonb_build_object('id', r.id));
            END LOOP;
        END IF;
        IF held THEN
            UPDATE public.wanted_tracks t SET discovered_at = NEW.discovered_at, duration_ms = NEW.duration_ms, external_id = NEW.external_id, fetched_at = NEW.fetched_at, isrc = NEW.isrc, normalized_title = NEW.normalized_title, primary_artist_id = NEW.primary_artist_id, recording_key = NEW.recording_key, release_year = NEW.release_year, resolve_attempts = NEW.resolve_attempts, resolve_error = NEW.resolve_error, resolve_locked_at = NEW.resolve_locked_at, resolve_next_run_at = NEW.resolve_next_run_at, source = NEW.source, status = NEW.status, title = NEW.title, track_id = NEW.track_id, updated_at = NEW.updated_at, work_key = NEW.work_key, work_normalizer_version = NEW.work_normalizer_version, work_reconciled_at = NEW.work_reconciled_at WHERE t.id = NEW.id;
            RETURN NULL;
        END IF;
        PERFORM pg_catalog.set_config('scd.merging', 'on', true);
        INSERT INTO public.wanted_tracks (discovered_at, duration_ms, external_id, fetched_at, id, isrc, normalized_title, primary_artist_id, recording_key, release_year, resolve_attempts, resolve_error, resolve_locked_at, resolve_next_run_at, source, status, title, track_id, updated_at, work_key, work_normalizer_version, work_reconciled_at)
        VALUES (NEW.discovered_at, NEW.duration_ms, NEW.external_id, NEW.fetched_at, NEW.id, NEW.isrc, NEW.normalized_title, NEW.primary_artist_id, NEW.recording_key, NEW.release_year, NEW.resolve_attempts, NEW.resolve_error, NEW.resolve_locked_at, NEW.resolve_next_run_at, NEW.source, NEW.status, NEW.title, NEW.track_id, NEW.updated_at, NEW.work_key, NEW.work_normalizer_version, NEW.work_reconciled_at)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS n = ROW_COUNT;
        PERFORM pg_catalog.set_config('scd.merging', '', true);
        IF n > 0 THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RAISE EXCEPTION 'scd: insert into public.wanted_tracks still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE OR DELETE ON public.wanted_tracks FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_wanted_tracks();
ALTER TABLE public.wanted_tracks ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.wanted_tracks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.wanted_tracks ENABLE REPLICA TRIGGER zzz_suppress;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
CREATE OR REPLACE FUNCTION public.scd_row_hash(tbl text, k jsonb)
RETURNS text
LANGUAGE plpgsql STABLE
AS $f$
BEGIN
    CASE tbl
        WHEN 'admin_catalog_requests' THEN
            RETURN public.scd_row_hash_admin_catalog_requests(k);
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

CREATE OR REPLACE FUNCTION public.scd_purge(tbl text, k jsonb)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    CASE tbl
        WHEN 'admin_catalog_requests' THEN
            PERFORM public.scd_purge_admin_catalog_requests(k);
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
        WHEN 'storage_event_repairs' THEN
            PERFORM public.scd_purge_storage_event_repairs(k);
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

DROP FUNCTION IF EXISTS public.scd_purge_intent(text, jsonb);
CREATE OR REPLACE FUNCTION public.scd_purge_intent(tbl text, k jsonb, seen bigint)
RETURNS boolean
LANGUAGE plpgsql
AS $f$
BEGIN
    CASE tbl
        WHEN 'soundcloud_connections' THEN
            RETURN public.scd_purge_intent_soundcloud_connections(k, seen);
        WHEN 'sync_queue' THEN
            RETURN public.scd_purge_intent_sync_queue(k, seen);
        WHEN 'user_followings' THEN
            RETURN public.scd_purge_intent_user_followings(k, seen);
        WHEN 'user_likes_playlists' THEN
            RETURN public.scd_purge_intent_user_likes_playlists(k, seen);
        WHEN 'user_likes_tracks' THEN
            RETURN public.scd_purge_intent_user_likes_tracks(k, seen);
        ELSE
            RAISE EXCEPTION 'public.scd_purge_intent: unknown table %', tbl;
    END CASE;
END
$f$;

CREATE OR REPLACE VIEW public.scd_counters AS
SELECT c.counter, c.tbl, coalesce(s.last_value, 0) AS n
FROM (VALUES
    ('conflict', 'cdn_tracks', 'scd_conflict_cdn_tracks'),
    ('conflict', 'discover_promoted', 'scd_conflict_discover_promoted'),
    ('conflict', 'disliked_tracks', 'scd_conflict_disliked_tracks'),
    ('conflict', 'oauth_apps', 'scd_conflict_oauth_apps'),
    ('conflict', 'sync_queue', 'scd_conflict_sync_queue'),
    ('core_col', 'artists', 'scd_core_col_artists'),
    ('core_col', 'lyrics_cache', 'scd_core_col_lyrics_cache'),
    ('core_col', 'playlists', 'scd_core_col_playlists'),
    ('guard', 'admin_catalog_requests', 'scd_guard_admin_catalog_requests'),
    ('guard', 'admin_maintenance_runs', 'scd_guard_admin_maintenance_runs'),
    ('guard', 'ai_resolver_budget', 'scd_guard_ai_resolver_budget'),
    ('guard', 'ai_resolver_cache', 'scd_guard_ai_resolver_cache'),
    ('guard', 'album_artists', 'scd_guard_album_artists'),
    ('guard', 'album_tracks', 'scd_guard_album_tracks'),
    ('guard', 'albums', 'scd_guard_albums'),
    ('guard', 'artist_attribution_revalidation', 'scd_guard_artist_attribution_revalidation'),
    ('guard', 'artist_colike', 'scd_guard_artist_colike'),
    ('guard', 'artist_coplay', 'scd_guard_artist_coplay'),
    ('guard', 'artist_sc_accounts', 'scd_guard_artist_sc_accounts'),
    ('guard', 'artist_socials', 'scd_guard_artist_socials'),
    ('guard', 'artists', 'scd_guard_artists'),
    ('guard', 'audio_index_wire_state', 'scd_guard_audio_index_wire_state'),
    ('guard', 'background_job_enqueues', 'scd_guard_background_job_enqueues'),
    ('guard', 'background_job_failures', 'scd_guard_background_job_failures'),
    ('guard', 'background_jobs', 'scd_guard_background_jobs'),
    ('guard', 'background_schedules', 'scd_guard_background_schedules'),
    ('guard', 'catalog_audience', 'scd_guard_catalog_audience'),
    ('guard', 'catalog_collection_cursors', 'scd_guard_catalog_collection_cursors'),
    ('guard', 'catalog_collection_seen', 'scd_guard_catalog_collection_seen'),
    ('guard', 'catalog_collection_sync', 'scd_guard_catalog_collection_sync'),
    ('guard', 'catalog_merge_state', 'scd_guard_catalog_merge_state'),
    ('guard', 'catalog_work_links', 'scd_guard_catalog_work_links'),
    ('guard', 'cmd_dead', 'scd_guard_cmd_dead'),
    ('guard', 'discover_tag_counts', 'scd_guard_discover_tag_counts'),
    ('guard', 'enrich_calibration', 'scd_guard_enrich_calibration'),
    ('guard', 'job_status', 'scd_guard_job_status'),
    ('guard', 'lyrics_cache', 'scd_guard_lyrics_cache'),
    ('guard', 'lyrics_embedding_wire_state', 'scd_guard_lyrics_embedding_wire_state'),
    ('guard', 'lyrics_lookup_backfill_state', 'scd_guard_lyrics_lookup_backfill_state'),
    ('guard', 'lyrics_lookup_state', 'scd_guard_lyrics_lookup_state'),
    ('guard', 'oauth_app_token_issuance_reservations', 'scd_guard_oauth_app_token_issuance_reservations'),
    ('guard', 'oauth_app_token_refresh_state', 'scd_guard_oauth_app_token_refresh_state'),
    ('guard', 'oauth_app_tokens', 'scd_guard_oauth_app_tokens'),
    ('guard', 'pipeline_event_receipts', 'scd_guard_pipeline_event_receipts'),
    ('guard', 'playlist_legacy_membership_intents', 'scd_guard_playlist_legacy_membership_intents'),
    ('guard', 'playlist_membership_operations', 'scd_guard_playlist_membership_operations'),
    ('guard', 'playlist_membership_state', 'scd_guard_playlist_membership_state'),
    ('guard', 'playlist_reconcile_runs', 'scd_guard_playlist_reconcile_runs'),
    ('guard', 'playlist_remote_observations', 'scd_guard_playlist_remote_observations'),
    ('guard', 'playlist_remote_snapshot_tracks', 'scd_guard_playlist_remote_snapshot_tracks'),
    ('guard', 'playlist_remote_snapshots', 'scd_guard_playlist_remote_snapshots'),
    ('guard', 'playlist_sweep_cursor', 'scd_guard_playlist_sweep_cursor'),
    ('guard', 'playlist_track_projection', 'scd_guard_playlist_track_projection'),
    ('guard', 'playlists', 'scd_guard_playlists'),
    ('guard', 'recommendation_quality_models', 'scd_guard_recommendation_quality_models'),
    ('guard', 'search_terms', 'scd_guard_search_terms'),
    ('guard', 'star_track_tiers', 'scd_guard_star_track_tiers'),
    ('guard', 'storage_event_repairs', 'scd_guard_storage_event_repairs'),
    ('guard', 'storage_event_state', 'scd_guard_storage_event_state'),
    ('guard', 'subscription_snapshot_state', 'scd_guard_subscription_snapshot_state'),
    ('guard', 'sync_queue_action_pauses', 'scd_guard_sync_queue_action_pauses'),
    ('guard', 'taste_model_versions', 'scd_guard_taste_model_versions'),
    ('guard', 'taste_schedule', 'scd_guard_taste_schedule'),
    ('guard', 'track_artists', 'scd_guard_track_artists'),
    ('guard', 'track_comments', 'scd_guard_track_comments'),
    ('guard', 'track_work_aliases', 'scd_guard_track_work_aliases'),
    ('guard', 'tracks', 'scd_guard_tracks'),
    ('guard', 'transcription_sync_versions', 'scd_guard_transcription_sync_versions'),
    ('guard', 'transcription_wire_state', 'scd_guard_transcription_wire_state'),
    ('guard', 'user_collection_sync', 'scd_guard_user_collection_sync'),
    ('guard', 'user_followers', 'scd_guard_user_followers'),
    ('guard', 'user_owned_playlists', 'scd_guard_user_owned_playlists'),
    ('guard', 'user_owned_tracks', 'scd_guard_user_owned_tracks'),
    ('guard', 'user_profiles', 'scd_guard_user_profiles'),
    ('guard', 'user_taste_vectors', 'scd_guard_user_taste_vectors'),
    ('guard', 'user_web_profiles', 'scd_guard_user_web_profiles'),
    ('guard', 'users', 'scd_guard_users'),
    ('guard', 'wanted_track_albums', 'scd_guard_wanted_track_albums'),
    ('guard', 'wanted_track_artists', 'scd_guard_wanted_track_artists'),
    ('guard', 'wanted_track_work_aliases', 'scd_guard_wanted_track_work_aliases'),
    ('guard', 'wanted_tracks', 'scd_guard_wanted_tracks'),
    ('guard_intent', 'sync_queue', 'scd_guard_intent_sync_queue'),
    ('guard_intent', 'user_followings', 'scd_guard_intent_user_followings'),
    ('guard_intent', 'user_likes_playlists', 'scd_guard_intent_user_likes_playlists'),
    ('guard_intent', 'user_likes_tracks', 'scd_guard_intent_user_likes_tracks'),
    ('guard_result', 'soundcloud_connections', 'scd_guard_result_soundcloud_connections'),
    ('guard_result', 'sync_queue', 'scd_guard_result_sync_queue'),
    ('guard_result', 'user_followings', 'scd_guard_result_user_followings'),
    ('guard_result', 'user_likes_playlists', 'scd_guard_result_user_likes_playlists'),
    ('guard_result', 'user_likes_tracks', 'scd_guard_result_user_likes_tracks'),
    ('orphan', 'oauth_app_request_cooldowns', 'scd_orphan_oauth_app_request_cooldowns'),
    ('orphan', 'sessions', 'scd_orphan_sessions'),
    ('orphan', 'soundcloud_connections', 'scd_orphan_soundcloud_connections'),
    ('orphan', 'track_artist_blocks', 'scd_orphan_track_artist_blocks'),
    ('purge_block', 'soundcloud_connections', 'scd_purge_block_soundcloud_connections'),
    ('purge_block', 'sync_queue', 'scd_purge_block_sync_queue'),
    ('purge_block', 'user_followings', 'scd_purge_block_user_followings'),
    ('purge_block', 'user_likes_playlists', 'scd_purge_block_user_likes_playlists'),
    ('purge_block', 'user_likes_tracks', 'scd_purge_block_user_likes_tracks'),
    ('resend', 'discover_promoted', 'scd_resend_discover_promoted'),
    ('resend', 'disliked_tracks', 'scd_resend_disliked_tracks'),
    ('resend', 'sc_egress_health', 'scd_resend_sc_egress_health'),
    ('resend', 'soundcloud_connections', 'scd_resend_soundcloud_connections'),
    ('resend', 'subscriptions', 'scd_resend_subscriptions'),
    ('resend', 'sync_queue', 'scd_resend_sync_queue'),
    ('resend', 'track_artist_blocks', 'scd_resend_track_artist_blocks'),
    ('resend', 'user_blocked_artists', 'scd_resend_user_blocked_artists'),
    ('resend', 'user_followings', 'scd_resend_user_followings'),
    ('resend', 'user_like_ranks', 'scd_resend_user_like_ranks'),
    ('resend', 'user_likes_playlists', 'scd_resend_user_likes_playlists'),
    ('resend', 'user_likes_tracks', 'scd_resend_user_likes_tracks'),
    ('resend', 'user_superlike_ranks', 'scd_resend_user_superlike_ranks'),
    ('resend', 'user_superlikes', 'scd_resend_user_superlikes'),
    ('restrict', 'oauth_apps', 'scd_restrict_oauth_apps')
) AS c (counter, tbl, seq)
LEFT JOIN pg_catalog.pg_sequences s ON s.schemaname = 'public' AND s.sequencename = c.seq;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
DO $d$
DECLARE
    r record;
BEGIN
    FOR r IN
        SELECT c.relname::text AS tbl, t.tgname::text AS name
        FROM pg_catalog.pg_trigger t
        JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid
        JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public' AND NOT t.tgisinternal AND t.tgname::text IN ('zz_core_col', 'zz_erase', 'zz_fk', 'zz_guard', 'zz_guard_intent', 'zz_guard_result', 'zz_max', 'zz_merge', 'zz_orphan', 'zz_owner', 'zz_purge_guard', 'zz_skip', 'zz_tomb', 'zz_tomb_check', 'zz_tomb_guard', 'zz_ver', 'zzz_suppress')
        EXCEPT VALUES
            ('admin_catalog_requests', 'zz_guard'),
            ('admin_maintenance_runs', 'zz_guard'),
            ('admin_maintenance_runs', 'zz_owner'),
            ('admin_maintenance_runs', 'zzz_suppress'),
            ('ai_resolver_budget', 'zz_guard'),
            ('ai_resolver_cache', 'zz_guard'),
            ('album_artists', 'zz_guard'),
            ('album_artists', 'zz_owner'),
            ('album_artists', 'zzz_suppress'),
            ('album_tracks', 'zz_guard'),
            ('album_tracks', 'zz_owner'),
            ('album_tracks', 'zzz_suppress'),
            ('albums', 'zz_guard'),
            ('albums', 'zz_owner'),
            ('albums', 'zzz_suppress'),
            ('artist_attribution_revalidation', 'zz_guard'),
            ('artist_attribution_revalidation', 'zz_owner'),
            ('artist_attribution_revalidation', 'zzz_suppress'),
            ('artist_colike', 'zz_guard'),
            ('artist_colike', 'zz_owner'),
            ('artist_colike', 'zzz_suppress'),
            ('artist_coplay', 'zz_guard'),
            ('artist_coplay', 'zz_owner'),
            ('artist_coplay', 'zzz_suppress'),
            ('artist_sc_accounts', 'zz_guard'),
            ('artist_sc_accounts', 'zz_owner'),
            ('artist_sc_accounts', 'zzz_suppress'),
            ('artist_socials', 'zz_guard'),
            ('artist_socials', 'zz_owner'),
            ('artist_socials', 'zzz_suppress'),
            ('artists', 'zz_core_col'),
            ('artists', 'zz_guard'),
            ('artists', 'zz_owner'),
            ('artists', 'zzz_suppress'),
            ('audio_index_wire_state', 'zz_guard'),
            ('background_job_enqueues', 'zz_guard'),
            ('background_job_failures', 'zz_guard'),
            ('background_job_failures', 'zz_owner'),
            ('background_job_failures', 'zzz_suppress'),
            ('background_jobs', 'zz_guard'),
            ('background_schedules', 'zz_guard'),
            ('catalog_audience', 'zz_guard'),
            ('catalog_audience', 'zz_owner'),
            ('catalog_audience', 'zzz_suppress'),
            ('catalog_collection_cursors', 'zz_guard'),
            ('catalog_collection_seen', 'zz_guard'),
            ('catalog_collection_sync', 'zz_guard'),
            ('catalog_collection_sync', 'zz_owner'),
            ('catalog_collection_sync', 'zzz_suppress'),
            ('catalog_merge_state', 'zz_guard'),
            ('catalog_work_links', 'zz_guard'),
            ('cdn_tracks', 'zz_merge'),
            ('cdn_tracks', 'zz_ver'),
            ('cdn_tracks', 'zzz_suppress'),
            ('cmd_dead', 'zz_guard'),
            ('diag_bundles', 'zz_skip'),
            ('diag_bundles', 'zzz_suppress'),
            ('diag_events', 'zz_skip'),
            ('diag_events', 'zzz_suppress'),
            ('diag_issue_days', 'zz_merge'),
            ('diag_issue_days', 'zz_ver'),
            ('diag_issue_days', 'zzz_suppress'),
            ('diag_issue_origins', 'zz_skip'),
            ('diag_issue_origins', 'zzz_suppress'),
            ('discover_promoted', 'zz_merge'),
            ('discover_promoted', 'zz_tomb'),
            ('discover_promoted', 'zz_tomb_check'),
            ('discover_promoted', 'zz_tomb_guard'),
            ('discover_promoted', 'zz_ver'),
            ('discover_promoted', 'zzz_suppress'),
            ('discover_settings', 'zz_merge'),
            ('discover_settings', 'zz_ver'),
            ('discover_settings', 'zzz_suppress'),
            ('discover_tag_counts', 'zz_guard'),
            ('discover_tag_counts', 'zz_owner'),
            ('discover_tag_counts', 'zzz_suppress'),
            ('disliked_tracks', 'zz_merge'),
            ('disliked_tracks', 'zz_tomb'),
            ('disliked_tracks', 'zz_tomb_check'),
            ('disliked_tracks', 'zz_tomb_guard'),
            ('disliked_tracks', 'zz_ver'),
            ('disliked_tracks', 'zzz_suppress'),
            ('enrich_calibration', 'zz_guard'),
            ('entity_reads', 'zz_max'),
            ('entity_reads', 'zzz_suppress'),
            ('featured_items', 'zz_merge'),
            ('featured_items', 'zz_tomb'),
            ('featured_items', 'zz_tomb_check'),
            ('featured_items', 'zz_ver'),
            ('featured_items', 'zzz_suppress'),
            ('job_status', 'zz_guard'),
            ('job_status', 'zz_owner'),
            ('job_status', 'zzz_suppress'),
            ('link_requests', 'zz_merge'),
            ('link_requests', 'zz_tomb'),
            ('link_requests', 'zz_tomb_check'),
            ('link_requests', 'zz_ver'),
            ('link_requests', 'zzz_suppress'),
            ('listening_history', 'zz_skip'),
            ('listening_history', 'zz_tomb'),
            ('listening_history', 'zz_tomb_check'),
            ('listening_history', 'zzz_suppress'),
            ('login_requests', 'zz_merge'),
            ('login_requests', 'zz_tomb'),
            ('login_requests', 'zz_tomb_check'),
            ('login_requests', 'zz_ver'),
            ('login_requests', 'zzz_suppress'),
            ('lyrics_cache', 'zz_core_col'),
            ('lyrics_cache', 'zz_guard'),
            ('lyrics_cache', 'zz_owner'),
            ('lyrics_cache', 'zzz_suppress'),
            ('lyrics_embedding_wire_state', 'zz_guard'),
            ('lyrics_lookup_backfill_state', 'zz_guard'),
            ('lyrics_lookup_state', 'zz_guard'),
            ('lyrics_lookup_state', 'zz_owner'),
            ('lyrics_lookup_state', 'zzz_suppress'),
            ('oauth_app_request_cooldowns', 'zz_merge'),
            ('oauth_app_request_cooldowns', 'zz_orphan'),
            ('oauth_app_request_cooldowns', 'zz_ver'),
            ('oauth_app_request_cooldowns', 'zzz_suppress'),
            ('oauth_app_token_issuance_reservations', 'zz_guard'),
            ('oauth_app_token_refresh_state', 'zz_guard'),
            ('oauth_app_tokens', 'zz_guard'),
            ('oauth_app_tokens', 'zz_owner'),
            ('oauth_app_tokens', 'zzz_suppress'),
            ('oauth_apps', 'zz_fk'),
            ('oauth_apps', 'zz_merge'),
            ('oauth_apps', 'zz_ver'),
            ('oauth_apps', 'zzz_suppress'),
            ('pipeline_event_receipts', 'zz_guard'),
            ('playlist_legacy_membership_intents', 'zz_guard'),
            ('playlist_legacy_membership_intents', 'zz_owner'),
            ('playlist_legacy_membership_intents', 'zzz_suppress'),
            ('playlist_membership_operations', 'zz_guard'),
            ('playlist_membership_operations', 'zz_owner'),
            ('playlist_membership_operations', 'zzz_suppress'),
            ('playlist_membership_state', 'zz_guard'),
            ('playlist_membership_state', 'zz_owner'),
            ('playlist_membership_state', 'zzz_suppress'),
            ('playlist_reconcile_runs', 'zz_guard'),
            ('playlist_remote_observations', 'zz_guard'),
            ('playlist_remote_observations', 'zz_owner'),
            ('playlist_remote_observations', 'zzz_suppress'),
            ('playlist_remote_snapshot_tracks', 'zz_guard'),
            ('playlist_remote_snapshots', 'zz_guard'),
            ('playlist_remote_snapshots', 'zz_owner'),
            ('playlist_remote_snapshots', 'zzz_suppress'),
            ('playlist_sweep_cursor', 'zz_guard'),
            ('playlist_track_projection', 'zz_guard'),
            ('playlist_track_projection', 'zz_owner'),
            ('playlist_track_projection', 'zzz_suppress'),
            ('playlists', 'zz_core_col'),
            ('playlists', 'zz_guard'),
            ('playlists', 'zz_owner'),
            ('playlists', 'zzz_suppress'),
            ('recommendation_quality_models', 'zz_guard'),
            ('repl_checksums', 'zz_max'),
            ('repl_checksums', 'zzz_suppress'),
            ('repl_tombstones', 'zz_erase'),
            ('repl_tombstones', 'zz_ver'),
            ('repl_tombstones', 'zzz_suppress'),
            ('sc_egress_health', 'zz_merge'),
            ('sc_egress_health', 'zz_tomb'),
            ('sc_egress_health', 'zz_tomb_check'),
            ('sc_egress_health', 'zz_tomb_guard'),
            ('sc_egress_health', 'zz_ver'),
            ('sc_egress_health', 'zzz_suppress'),
            ('sc_track_counters', 'zz_max'),
            ('sc_track_counters', 'zzz_suppress'),
            ('search_terms', 'zz_guard'),
            ('search_terms', 'zz_owner'),
            ('search_terms', 'zzz_suppress'),
            ('sessions', 'zz_merge'),
            ('sessions', 'zz_orphan'),
            ('sessions', 'zz_tomb'),
            ('sessions', 'zz_tomb_check'),
            ('sessions', 'zz_ver'),
            ('sessions', 'zzz_suppress'),
            ('soundcloud_connections', 'zz_fk'),
            ('soundcloud_connections', 'zz_guard_result'),
            ('soundcloud_connections', 'zz_merge'),
            ('soundcloud_connections', 'zz_orphan'),
            ('soundcloud_connections', 'zz_purge_guard'),
            ('soundcloud_connections', 'zz_ver'),
            ('soundcloud_connections', 'zzz_suppress'),
            ('star_track_tiers', 'zz_guard'),
            ('storage_event_repairs', 'zz_guard'),
            ('storage_event_state', 'zz_guard'),
            ('subscription_snapshot_state', 'zz_guard'),
            ('subscriptions', 'zz_merge'),
            ('subscriptions', 'zz_tomb'),
            ('subscriptions', 'zz_tomb_check'),
            ('subscriptions', 'zz_tomb_guard'),
            ('subscriptions', 'zz_ver'),
            ('subscriptions', 'zzz_suppress'),
            ('sync_queue', 'zz_guard_intent'),
            ('sync_queue', 'zz_guard_result'),
            ('sync_queue', 'zz_merge'),
            ('sync_queue', 'zz_purge_guard'),
            ('sync_queue', 'zz_ver'),
            ('sync_queue', 'zzz_suppress'),
            ('sync_queue_action_pauses', 'zz_guard'),
            ('sync_queue_action_pauses', 'zz_owner'),
            ('sync_queue_action_pauses', 'zzz_suppress'),
            ('taste_model_versions', 'zz_guard'),
            ('taste_model_versions', 'zz_owner'),
            ('taste_model_versions', 'zzz_suppress'),
            ('taste_schedule', 'zz_guard'),
            ('track_artist_blocks', 'zz_merge'),
            ('track_artist_blocks', 'zz_orphan'),
            ('track_artist_blocks', 'zz_tomb'),
            ('track_artist_blocks', 'zz_tomb_check'),
            ('track_artist_blocks', 'zz_tomb_guard'),
            ('track_artist_blocks', 'zz_ver'),
            ('track_artist_blocks', 'zzz_suppress'),
            ('track_artists', 'zz_guard'),
            ('track_artists', 'zz_owner'),
            ('track_artists', 'zzz_suppress'),
            ('track_comments', 'zz_guard'),
            ('track_comments', 'zz_owner'),
            ('track_comments', 'zzz_suppress'),
            ('track_work_aliases', 'zz_guard'),
            ('tracks', 'zz_guard'),
            ('tracks', 'zz_owner'),
            ('tracks', 'zzz_suppress'),
            ('transcription_sync_versions', 'zz_guard'),
            ('transcription_wire_state', 'zz_guard'),
            ('user_auras', 'zz_merge'),
            ('user_auras', 'zz_ver'),
            ('user_auras', 'zzz_suppress'),
            ('user_blocked_artists', 'zz_merge'),
            ('user_blocked_artists', 'zz_tomb'),
            ('user_blocked_artists', 'zz_tomb_check'),
            ('user_blocked_artists', 'zz_tomb_guard'),
            ('user_blocked_artists', 'zz_ver'),
            ('user_blocked_artists', 'zzz_suppress'),
            ('user_collection_sync', 'zz_guard'),
            ('user_events', 'zz_skip'),
            ('user_events', 'zzz_suppress'),
            ('user_followers', 'zz_guard'),
            ('user_followers', 'zz_owner'),
            ('user_followers', 'zzz_suppress'),
            ('user_followings', 'zz_guard_intent'),
            ('user_followings', 'zz_guard_result'),
            ('user_followings', 'zz_merge'),
            ('user_followings', 'zz_purge_guard'),
            ('user_followings', 'zz_ver'),
            ('user_followings', 'zzz_suppress'),
            ('user_like_ranks', 'zz_merge'),
            ('user_like_ranks', 'zz_tomb'),
            ('user_like_ranks', 'zz_tomb_check'),
            ('user_like_ranks', 'zz_tomb_guard'),
            ('user_like_ranks', 'zz_ver'),
            ('user_like_ranks', 'zzz_suppress'),
            ('user_likes_playlists', 'zz_guard_intent'),
            ('user_likes_playlists', 'zz_guard_result'),
            ('user_likes_playlists', 'zz_merge'),
            ('user_likes_playlists', 'zz_purge_guard'),
            ('user_likes_playlists', 'zz_ver'),
            ('user_likes_playlists', 'zzz_suppress'),
            ('user_likes_tracks', 'zz_guard_intent'),
            ('user_likes_tracks', 'zz_guard_result'),
            ('user_likes_tracks', 'zz_merge'),
            ('user_likes_tracks', 'zz_purge_guard'),
            ('user_likes_tracks', 'zz_ver'),
            ('user_likes_tracks', 'zzz_suppress'),
            ('user_owned_playlists', 'zz_guard'),
            ('user_owned_playlists', 'zz_owner'),
            ('user_owned_playlists', 'zzz_suppress'),
            ('user_owned_tracks', 'zz_guard'),
            ('user_owned_tracks', 'zz_owner'),
            ('user_owned_tracks', 'zzz_suppress'),
            ('user_profiles', 'zz_guard'),
            ('user_profiles', 'zz_owner'),
            ('user_profiles', 'zzz_suppress'),
            ('user_superlike_ranks', 'zz_merge'),
            ('user_superlike_ranks', 'zz_tomb'),
            ('user_superlike_ranks', 'zz_tomb_check'),
            ('user_superlike_ranks', 'zz_tomb_guard'),
            ('user_superlike_ranks', 'zz_ver'),
            ('user_superlike_ranks', 'zzz_suppress'),
            ('user_superlikes', 'zz_merge'),
            ('user_superlikes', 'zz_tomb'),
            ('user_superlikes', 'zz_tomb_check'),
            ('user_superlikes', 'zz_tomb_guard'),
            ('user_superlikes', 'zz_ver'),
            ('user_superlikes', 'zzz_suppress'),
            ('user_taste_vectors', 'zz_guard'),
            ('user_taste_vectors', 'zz_owner'),
            ('user_taste_vectors', 'zzz_suppress'),
            ('user_web_profiles', 'zz_guard'),
            ('user_web_profiles', 'zz_owner'),
            ('user_web_profiles', 'zzz_suppress'),
            ('users', 'zz_guard'),
            ('users', 'zz_owner'),
            ('users', 'zzz_suppress'),
            ('wanted_track_albums', 'zz_guard'),
            ('wanted_track_albums', 'zz_owner'),
            ('wanted_track_albums', 'zzz_suppress'),
            ('wanted_track_artists', 'zz_guard'),
            ('wanted_track_artists', 'zz_owner'),
            ('wanted_track_artists', 'zzz_suppress'),
            ('wanted_track_work_aliases', 'zz_guard'),
            ('wanted_tracks', 'zz_guard'),
            ('wanted_tracks', 'zz_owner'),
            ('wanted_tracks', 'zzz_suppress')
        ORDER BY 1, 2
    LOOP
        EXECUTE format('DROP TRIGGER %I ON public.%I', r.name, r.tbl);
    END LOOP;
END
$d$;
DROP FUNCTION IF EXISTS public.scd_merge_repl_tombstones();
COMMIT;

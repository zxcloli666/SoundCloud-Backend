INSERT INTO oauth_apps (id)
VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'),
       ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb');

INSERT INTO sessions (
    id,
    access_token,
    refresh_token,
    expires_at,
    scope,
    soundcloud_user_id,
    username,
    oauth_app_id,
    created_at,
    updated_at
)
VALUES
    ('10000000-0000-4000-8000-000000000001', 'access-shared-1', 'refresh-shared', now() - interval '2 hours', 'non-expiring', 'soundcloud:users:42', 'listener', 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', now() - interval '30 days', now() - interval '2 hours'),
    ('10000000-0000-4000-8000-000000000002', 'access-shared-2', 'refresh-shared', now() - interval '1 hour', 'non-expiring', '42', 'listener', 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', now() - interval '20 days', now() - interval '1 hour'),
    ('10000000-0000-4000-8000-000000000003', 'access-grant-2', 'refresh-grant-2', now() + interval '1 hour', 'non-expiring', '42', 'listener', 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', now() - interval '10 days', now() - interval '10 minutes'),
    ('10000000-0000-4000-8000-000000000004', 'access-app-b', 'refresh-app-b', now() + interval '2 hours', 'non-expiring', 'soundcloud:users:42', 'listener', 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', now() - interval '5 days', now() - interval '5 minutes'),
    ('10000000-0000-4000-8000-000000000005', 'access-old', 'refresh-old', now() - interval '365 days', 'non-expiring', '99', 'offline-user', 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', now() - interval '500 days', now() - interval '365 days'),
    ('10000000-0000-4000-8000-000000000006', 'access-orphan', 'refresh-orphan', now() - interval '2 days', 'non-expiring', NULL, NULL, NULL, now() - interval '3 days', now() - interval '2 days'),
    ('10000000-0000-4000-8000-000000000007', '', '', now() - interval '2 days', '', '77', NULL, NULL, now() - interval '3 days', now() - interval '2 days'),
    ('10000000-0000-4000-8000-000000000008', 'access-missing-app', 'refresh-missing-app', now() + interval '1 hour', 'non-expiring', '88', 'missing-app', 'not-a-uuid', now() - interval '2 days', now() - interval '1 hour');

INSERT INTO background_schedules (kind)
VALUES ('auth.reap_sessions');

INSERT INTO background_jobs (id, kind, lease_id)
VALUES ('20000000-0000-4000-8000-000000000001', 'auth.reap_sessions', NULL),
       ('20000000-0000-4000-8000-000000000002', 'auth.reap_sessions', '30000000-0000-4000-8000-000000000001'),
       ('20000000-0000-4000-8000-000000000003', 'discover.aggregates', NULL);

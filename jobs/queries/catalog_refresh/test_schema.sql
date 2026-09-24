CREATE TABLE users (
    sc_user_id text PRIMARY KEY,
    urn text NOT NULL UNIQUE,
    username text NOT NULL,
    username_normalized text NOT NULL,
    full_name text,
    first_name text,
    last_name text,
    permalink text,
    permalink_url text,
    avatar_url text,
    country varchar(8),
    city text,
    description text,
    verified boolean NOT NULL DEFAULT false,
    followers_count bigint,
    followings_count bigint,
    tracks_count bigint,
    playlists_count bigint,
    reposts_count bigint,
    comments_count bigint,
    kind varchar(16),
    sc_created_at timestamptz,
    sc_last_modified timestamptz,
    sc_synced_at timestamptz NOT NULL DEFAULT now(),
    sc_observation bigint NOT NULL DEFAULT 0,
    last_read_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE FUNCTION sc_counter_drifted(old bigint, new bigint)
RETURNS boolean LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
SELECT new IS NOT NULL AND (old IS NULL OR abs(new - old) > GREATEST(5, abs(old) / 20))
$$;
CREATE TABLE user_profiles (
    soundcloud_user_id text PRIMARY KEY,
    profile_json jsonb NOT NULL,
    synced_at timestamp NOT NULL,
    sc_observation bigint NOT NULL DEFAULT 0
);
CREATE TABLE background_jobs (
    id uuid PRIMARY KEY,
    lease_id uuid,
    lease_generation bigint,
    generation bigint NOT NULL,
    lease_expires_at timestamptz
);
CREATE SEQUENCE catalog_metadata_clock AS bigint;

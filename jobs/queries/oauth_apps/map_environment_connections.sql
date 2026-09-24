UPDATE soundcloud_connections
SET oauth_app_id = $1,
    uses_environment_oauth_app = false,
    updated_at = now()
WHERE uses_environment_oauth_app

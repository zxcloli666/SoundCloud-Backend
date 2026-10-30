use sqlx::PgConnection;

use crate::error::AppResult;

#[derive(Debug, Clone, Copy)]
pub struct WantedMirror {
    pub table: &'static str,
    pub key_col: &'static str,
}

pub const LIKES_TRACKS: WantedMirror = WantedMirror {
    table: "user_likes_tracks",
    key_col: "sc_track_id",
};
pub const LIKES_PLAYLISTS: WantedMirror = WantedMirror {
    table: "user_likes_playlists",
    key_col: "playlist_urn",
};
pub const FOLLOWINGS: WantedMirror = WantedMirror {
    table: "user_followings",
    key_col: "target_user_urn",
};

pub async fn set_wanted(
    connection: &mut PgConnection,
    m: WantedMirror,
    user_id: &str,
    key: &str,
) -> AppResult<bool> {
    let user_id = crate::common::sc_ids::extract_sc_id(user_id);
    let sql = format!(
        "WITH before AS ( \
             SELECT wanted_state FROM {table} WHERE user_id = $1 AND {key_col} = $2 \
         ) \
         INSERT INTO {table} (user_id, {key_col}, wanted_state) \
         VALUES ($1, $2, true) \
         ON CONFLICT (user_id, {key_col}) DO UPDATE SET \
             wanted_state = true, \
             created_at = CASE WHEN {table}.wanted_state THEN {table}.created_at ELSE now() END, \
             wanted_v = public.scd_hlc({table}.wanted_v) \
         RETURNING NOT coalesce((SELECT wanted_state FROM before), false)",
        table = m.table,
        key_col = m.key_col,
    );
    let flipped: bool = sqlx::query_scalar(&sql)
        .bind(user_id)
        .bind(key)
        .fetch_one(connection)
        .await?;
    Ok(flipped)
}

pub async fn clear_wanted(
    connection: &mut PgConnection,
    m: WantedMirror,
    user_id: &str,
    key: &str,
) -> AppResult<()> {
    let user_id = crate::common::sc_ids::extract_sc_id(user_id);
    let sql = format!(
        "INSERT INTO {table} (user_id, {key_col}, wanted_state) \
         VALUES ($1, $2, false) \
         ON CONFLICT (user_id, {key_col}) DO UPDATE SET \
             wanted_state = false, \
             wanted_v = public.scd_hlc({table}.wanted_v)",
        table = m.table,
        key_col = m.key_col,
    );
    sqlx::query(&sql)
        .bind(user_id)
        .bind(key)
        .execute(connection)
        .await?;
    Ok(())
}

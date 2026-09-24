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
) -> AppResult<()> {
    let user_id = crate::common::sc_ids::extract_sc_id(user_id);
    let sql = format!(
        "INSERT INTO {table} (user_id, {key_col}, wanted_state, progress) \
         VALUES ($1, $2, true, true) \
         ON CONFLICT (user_id, {key_col}) DO UPDATE SET \
             wanted_state = true, \
             progress = CASE WHEN {table}.wanted_state = false \
                             THEN false \
                             ELSE {table}.progress END, \
             created_at = now()",
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

pub async fn clear_wanted(
    connection: &mut PgConnection,
    m: WantedMirror,
    user_id: &str,
    key: &str,
) -> AppResult<()> {
    let user_id = crate::common::sc_ids::extract_sc_id(user_id);
    let select_sql = format!(
        "SELECT progress, wanted_state FROM {table} WHERE user_id = $1 AND {key_col} = $2",
        table = m.table,
        key_col = m.key_col,
    );
    let row: Option<(bool, bool)> = sqlx::query_as(&select_sql)
        .bind(user_id)
        .bind(key)
        .fetch_optional(&mut *connection)
        .await?;
    match row {
        None | Some((_, false)) => Ok(()),
        Some((true, true)) => {
            let sql = format!(
                "DELETE FROM {table} WHERE user_id = $1 AND {key_col} = $2",
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
        Some((_, true)) => {
            let sql = format!(
                "UPDATE {table} SET wanted_state = false, progress = true \
                 WHERE user_id = $1 AND {key_col} = $2",
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
    }
}

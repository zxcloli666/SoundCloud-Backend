use axum::http::StatusCode;
use serde::Deserialize;
use sqlx::{FromRow, PgPool};

use crate::error::{AppError, AppResult};
use crate::modules::lyrics::service::LyricsResponse;

pub const MAX_TEXT_BYTES: usize = 64 * 1024;
pub const MAX_PER_USER: i64 = 5000;

#[derive(Debug, Default, Deserialize)]
pub struct OwnLyricsInput {
    #[serde(default, rename = "plainText")]
    pub plain_text: Option<String>,
    #[serde(default, rename = "syncedLrc")]
    pub synced_lrc: Option<String>,
    #[serde(default, rename = "lrclibId")]
    pub lrclib_id: Option<i64>,
}

#[derive(Debug, FromRow)]
struct OwnRow {
    plain_text: Option<String>,
    synced_lrc: Option<String>,
    origin: String,
    lrclib_id: Option<i64>,
}

#[derive(Debug, PartialEq, Eq)]
struct Checked {
    plain_text: String,
    synced_lrc: String,
    origin: &'static str,
    lrclib_id: i64,
}

pub async fn read(
    pg: &PgPool,
    sc_user_id: &str,
    sc_track_id: &str,
) -> AppResult<Option<LyricsResponse>> {
    let row = sqlx::query_file_as!(
        OwnRow,
        "queries/lyrics/own/get.sql",
        sc_user_id,
        sc_track_id
    )
    .fetch_optional(pg)
    .await?;
    Ok(row.map(|row| response(sc_track_id, row)))
}

pub async fn put(
    pg: &PgPool,
    sc_user_id: &str,
    sc_track_id: &str,
    input: OwnLyricsInput,
) -> AppResult<LyricsResponse> {
    let checked = check(input)?;
    let written = sqlx::query_file!(
        "queries/lyrics/own/put.sql",
        sc_user_id,
        sc_track_id,
        checked.plain_text,
        checked.synced_lrc,
        checked.origin,
        checked.lrclib_id,
        MAX_PER_USER
    )
    .execute(pg)
    .await?;
    if written.rows_affected() == 0 {
        return Err(AppError::coded(
            StatusCode::CONFLICT,
            "own_lyrics_limit",
            format!("up to {MAX_PER_USER} tracks can carry your own lyrics"),
        ));
    }
    read(pg, sc_user_id, sc_track_id)
        .await?
        .ok_or_else(|| AppError::internal("own lyrics vanished after the write"))
}

pub async fn remove(pg: &PgPool, sc_user_id: &str, sc_track_id: &str) -> AppResult<()> {
    sqlx::query_file!("queries/lyrics/own/delete.sql", sc_user_id, sc_track_id)
        .execute(pg)
        .await?;
    Ok(())
}

fn response(sc_track_id: &str, row: OwnRow) -> LyricsResponse {
    LyricsResponse {
        sc_track_id: Some(sc_track_id.to_owned()),
        synced_lrc: row.synced_lrc,
        synced_words: None,
        plain_text: row.plain_text,
        source: if row.origin == "lrclib" {
            "lrclib"
        } else {
            "user"
        }
        .to_owned(),
        language: None,
        language_confidence: None,
        status: "found".to_owned(),
        own: true,
        lrclib_id: row.lrclib_id,
    }
}

fn check(input: OwnLyricsInput) -> AppResult<Checked> {
    let plain_text = text(input.plain_text)?;
    let synced_lrc = text(input.synced_lrc)?;
    if plain_text.is_empty() && synced_lrc.is_empty() {
        return Err(invalid("lyrics_empty", "lyrics are empty".to_owned()));
    }
    if !synced_lrc.is_empty() {
        if let Some(line) = bad_lrc_line(&synced_lrc) {
            return Err(invalid(
                "lyrics_invalid_lrc",
                format!("line {line} is not an LRC line"),
            ));
        }
        if timed_text(&synced_lrc).next().is_none() {
            return Err(invalid(
                "lyrics_invalid_lrc",
                "no timed line has words".to_owned(),
            ));
        }
    }
    let (origin, lrclib_id) = match input.lrclib_id {
        None => ("pasted", 0),
        Some(id) if id > 0 => ("lrclib", id),
        Some(_) => {
            return Err(AppError::bad_request("lrclibId must be positive"));
        }
    };
    Ok(Checked {
        plain_text,
        synced_lrc,
        origin,
        lrclib_id,
    })
}

fn text(value: Option<String>) -> AppResult<String> {
    let value = value
        .unwrap_or_default()
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let value = value.trim();
    if value.contains('\0') {
        return Err(AppError::bad_request("lyrics must not contain NUL"));
    }
    if value.len() > MAX_TEXT_BYTES {
        return Err(invalid(
            "lyrics_too_long",
            format!("lyrics are over {MAX_TEXT_BYTES} bytes"),
        ));
    }
    Ok(value.to_owned())
}

fn invalid(code: &'static str, message: String) -> AppError {
    AppError::coded(StatusCode::UNPROCESSABLE_ENTITY, code, message)
}

fn bad_lrc_line(lrc: &str) -> Option<usize> {
    lrc.lines()
        .map(str::trim)
        .position(|line| !line.is_empty() && !is_meta_tag(line) && after_time_tags(line).is_none())
        .map(|index| index + 1)
}

fn timed_text(lrc: &str) -> impl Iterator<Item = &str> {
    lrc.lines()
        .filter_map(|line| after_time_tags(line.trim()))
        .filter(|text| !text.trim().is_empty())
}

fn after_time_tags(line: &str) -> Option<&str> {
    let mut rest = after_time_tag(line)?;
    while let Some(next) = after_time_tag(rest.trim_start()) {
        rest = next;
    }
    Some(rest)
}

fn after_time_tag(line: &str) -> Option<&str> {
    let body = line.strip_prefix('[')?;
    let (tag, rest) = body.split_once(']')?;
    let (minutes, rest_of_tag) = tag.split_once(':')?;
    let (seconds, fraction) = match rest_of_tag.split_once(['.', ':']) {
        Some((seconds, fraction)) => (seconds, Some(fraction)),
        None => (rest_of_tag, None),
    };
    let valid = digits(minutes, 1, 3)
        && digits(seconds, 2, 2)
        && seconds < "60"
        && fraction.is_none_or(|fraction| digits(fraction, 1, 3));
    valid.then_some(rest)
}

fn is_meta_tag(line: &str) -> bool {
    let Some(body) = line
        .strip_prefix('[')
        .and_then(|body| body.strip_suffix(']'))
    else {
        return false;
    };
    let Some((key, _)) = body.split_once(':') else {
        return false;
    };
    !key.is_empty()
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic() || byte == b'#')
}

fn digits(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
#[path = "own_tests.rs"]
mod tests;

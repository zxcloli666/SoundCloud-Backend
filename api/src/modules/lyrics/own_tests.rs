use super::*;
use crate::modules::lyrics::LyricsService;

fn pasted(text: &str) -> OwnLyricsInput {
    OwnLyricsInput {
        plain_text: Some(text.to_owned()),
        ..OwnLyricsInput::default()
    }
}

fn synced(lrc: &str) -> OwnLyricsInput {
    OwnLyricsInput {
        synced_lrc: Some(lrc.to_owned()),
        ..OwnLyricsInput::default()
    }
}

fn code(error: AppError) -> &'static str {
    error.public_code()
}

#[test]
fn an_lrc_with_tags_blank_lines_and_stacked_stamps_is_accepted() {
    let lrc = "[ar:Someone]\r\n[offset:+200]\n\n[00:01.00] first\n[00:05.5][01:10.250] chorus\n[00:09]\n[123:59:99] late";

    let checked = check(synced(lrc)).unwrap();

    assert_eq!(checked.origin, "pasted");
    assert!(!checked.synced_lrc.contains('\r'));
}

#[test]
fn an_untimed_line_inside_lrc_names_its_line() {
    let error = check(synced("[00:01.00] first\nsecond without a stamp")).unwrap_err();

    assert_eq!(
        error.to_string(),
        "lyrics_invalid_lrc: line 2 is not an LRC line"
    );
}

#[test]
fn impossible_stamps_are_refused() {
    for lrc in [
        "[00:60.00] sixty",
        "[0:1.00] short",
        "[00:01.0000] long",
        "[aa:01] x",
    ] {
        assert_eq!(
            code(check(synced(lrc)).unwrap_err()),
            "lyrics_invalid_lrc",
            "{lrc}"
        );
    }
}

#[test]
fn lrc_without_a_single_timed_word_is_refused() {
    assert_eq!(
        code(check(synced("[ti:Song]\n[00:01.00]\n[00:02.00]  ")).unwrap_err()),
        "lyrics_invalid_lrc"
    );
}

#[test]
fn empty_huge_and_nul_texts_are_refused() {
    assert_eq!(code(check(pasted("  \n ")).unwrap_err()), "lyrics_empty");
    assert_eq!(
        code(check(pasted(&"a".repeat(MAX_TEXT_BYTES + 1))).unwrap_err()),
        "lyrics_too_long"
    );
    assert_eq!(
        check(pasted("a\0b")).unwrap_err().status(),
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn an_lrclib_pick_keeps_its_record_id() {
    let checked = check(OwnLyricsInput {
        plain_text: Some("words".to_owned()),
        synced_lrc: Some("[00:01.00] words".to_owned()),
        lrclib_id: Some(42),
    })
    .unwrap();

    assert_eq!((checked.origin, checked.lrclib_id), ("lrclib", 42));
    assert!(
        check(OwnLyricsInput {
            lrclib_id: Some(0),
            ..pasted("words")
        })
        .is_err()
    );
}

async fn seed_cached(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms)
         VALUES ('42', 'soundcloud:tracks:42', 'Track', 'track', 120000)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO lyrics_cache (sc_track_id, source, plain_text, language)
         VALUES ('42', 'lrclib', 'the found words', 'en')",
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn own_lyrics_win_for_their_owner_only(pool: PgPool) -> anyhow::Result<()> {
    seed_cached(&pool).await?;
    let lyrics = LyricsService::new(pool.clone(), false);

    let saved = put(&pool, "17", "42", pasted("my own words")).await?;
    let mine = lyrics.ensure_lyrics("17", "soundcloud:tracks:42").await?;
    let theirs = lyrics.ensure_lyrics("18", "42").await?;

    assert!(saved.own && mine.own);
    assert_eq!(mine.plain_text.as_deref(), Some("my own words"));
    assert_eq!(mine.source, "user");
    assert!(!theirs.own);
    assert_eq!(theirs.plain_text.as_deref(), Some("the found words"));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_second_save_replaces_the_first_and_delete_brings_back_the_automatic(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_cached(&pool).await?;
    let lyrics = LyricsService::new(pool.clone(), false);
    put(&pool, "17", "42", pasted("first")).await?;

    let replaced = put(
        &pool,
        "17",
        "42",
        OwnLyricsInput {
            plain_text: Some("picked".to_owned()),
            synced_lrc: Some("[00:01.00] picked".to_owned()),
            lrclib_id: Some(9001),
        },
    )
    .await?;
    assert_eq!(replaced.source, "lrclib");
    assert_eq!(replaced.lrclib_id, Some(9001));
    assert_eq!(replaced.synced_lrc.as_deref(), Some("[00:01.00] picked"));

    remove(&pool, "17", "42").await?;
    let back = lyrics.ensure_lyrics("17", "42").await?;
    assert!(!back.own);
    assert_eq!(back.plain_text.as_deref(), Some("the found words"));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn own_lyrics_show_even_where_no_automatic_lookup_exists(pool: PgPool) -> anyhow::Result<()> {
    let lyrics = LyricsService::new(pool.clone(), true);
    put(&pool, "17", "77", synced("[00:01.00] only mine")).await?;

    let answer = lyrics.ensure_lyrics("17", "77").await?;

    assert_eq!(answer.status, "found");
    assert_eq!(answer.synced_lrc.as_deref(), Some("[00:01.00] only mine"));
    assert!(answer.plain_text.is_none());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_per_user_cap_blocks_new_tracks_but_not_edits(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO user_lyrics (soundcloud_user_id, sc_track_id, plain_text, origin)
         SELECT '17', g::text, 'words', 'pasted' FROM generate_series(1, $1) g",
    )
    .bind(MAX_PER_USER as i32)
    .execute(&pool)
    .await?;

    let refused = put(&pool, "17", "999999", pasted("one more"))
        .await
        .unwrap_err();
    let edited = put(&pool, "17", "1", pasted("edited")).await?;
    let other_user = put(&pool, "18", "999999", pasted("fine")).await?;

    assert_eq!(refused.public_code(), "own_lyrics_limit");
    assert_eq!(edited.plain_text.as_deref(), Some("edited"));
    assert!(other_user.own);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_schema_refuses_rows_the_api_never_writes(pool: PgPool) -> anyhow::Result<()> {
    for (statement, constraint) in [
        (
            "INSERT INTO user_lyrics (soundcloud_user_id, sc_track_id, plain_text, origin) VALUES ('1', '2', '  ', 'pasted')",
            "user_lyrics_text_present",
        ),
        (
            "INSERT INTO user_lyrics (soundcloud_user_id, sc_track_id, plain_text, origin) VALUES ('1', '2', 'x', 'lrclib')",
            "user_lyrics_origin_ck",
        ),
        (
            "INSERT INTO user_lyrics (soundcloud_user_id, sc_track_id, plain_text, origin) VALUES ('1', '2', repeat('x', 65537), 'pasted')",
            "user_lyrics_size_ck",
        ),
    ] {
        let refused = sqlx::query(statement).execute(&pool).await.unwrap_err();
        assert!(refused.to_string().contains(constraint), "{refused}");
    }
    Ok(())
}

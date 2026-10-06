use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};

use super::terms::TermRow;

const BIG_TABLES: &[&str] = &["tracks", "users", "playlists", "lyrics_cache"];

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, metadata_artist, uploader_username,
                             uploader_sc_user_id, duration_ms, play_count_sc, sharing)
         SELECT n::text, 'soundcloud:tracks:' || n,
                (ARRAY['love', 'night', 'drive', 'rain', 'fire'])[1 + n % 5] || ' song ' || n,
                (ARRAY['love', 'night', 'drive', 'rain', 'fire'])[1 + n % 5] || ' song ' || n,
                'artist' || (n % 700), 'user' || (n % 900), (n % 900)::text,
                100000 + n, n, CASE WHEN n % 50 = 0 THEN 'private' ELSE 'public' END
         FROM generate_series(1, 60000) n;
         INSERT INTO users (sc_user_id, urn, username, username_normalized, full_name, followers_count)
         SELECT n::text, 'soundcloud:users:' || n, 'user' || n, 'user' || n, 'Name ' || n, n
         FROM generate_series(1, 20000) n;
         INSERT INTO playlists (sc_playlist_id, urn, title, title_normalized, owner_sc_user_id, owner_username)
         SELECT n::text, 'soundcloud:playlists:' || n, 'mix ' || n, 'mix ' || n, (n % 900)::text, 'user' || (n % 900)
         FROM generate_series(1, 20000) n;
         INSERT INTO artists (name, normalized_name, source, track_count_primary)
         SELECT 'artist' || n, 'artist' || n, 'test', 1 FROM generate_series(1, 5000) n;
         INSERT INTO albums (title, normalized_title, source, type, track_count)
         SELECT 'record ' || n, 'record ' || n, 'test', 'album', 3 FROM generate_series(1, 5000) n;
         INSERT INTO lyrics_cache (sc_track_id, plain_text, source, plain_source)
         SELECT n::text, 'line one ' || n || E'\\nforever ' || (n % 100) || E' heart\\nwe are', 'lrclib', 'lrclib'
         FROM generate_series(1, 20000) n;
         REFRESH MATERIALIZED VIEW search_terms;
         ANALYZE tracks, users, playlists, artists, albums, lyrics_cache, search_terms;",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn configured(pool: &PgPool) -> anyhow::Result<Transaction<'static, Postgres>> {
    let mut tx = pool.begin().await?;
    sqlx::raw_sql(include_str!("../../../queries/search/configure.sql"))
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

fn assert_bounded(plan: &str, what: &str) {
    let scanned = seq_scans(plan);
    for table in BIG_TABLES {
        assert!(
            !scanned.iter().any(|name| name == table),
            "{what} scans {table}: {plan}"
        );
    }
    assert!(
        !plan.contains("tracks_public_popular_idx"),
        "{what} walks the popularity index: {plan}"
    );
}

fn seq_scans(plan: &str) -> Vec<String> {
    let json: Value = serde_json::from_str(plan).unwrap_or(Value::Null);
    let mut found = Vec::new();
    collect(&json, &mut found);
    found
}

fn collect(node: &Value, found: &mut Vec<String>) {
    match node {
        Value::Array(items) => items.iter().for_each(|item| collect(item, found)),
        Value::Object(map) => {
            if map.get("Node Type").and_then(Value::as_str) == Some("Seq Scan")
                && let Some(relation) = map.get("Relation Name").and_then(Value::as_str)
            {
                found.push(relation.to_owned());
            }
            map.values().for_each(|value| collect(value, found));
        }
        _ => {}
    }
}

macro_rules! explain {
    ($tx:expr, $file:literal $(, $bind:expr)* $(,)?) => {{
        let sql = format!("EXPLAIN (FORMAT JSON) {}", include_str!($file));
        let plan: Value = sqlx::query_scalar(&sql)
            $(.bind($bind))*
            .fetch_one(&mut *$tx)
            .await?;
        plan.to_string()
    }};
}

#[sqlx::test(migrations = "./migrations")]
async fn every_catalog_query_stays_on_its_indexes(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool).await?;
    let mut tx = configured(&pool).await?;

    for (what, strict, loose, q, owner, index) in [
        (
            "a common token",
            "'love'",
            None,
            "love",
            None,
            "tracks_search_doc_gin",
        ),
        (
            "a rare multi-token",
            "'song' & '4242'",
            Some("'song' | '4242'"),
            "song 4242",
            None,
            "tracks_search_doc_gin",
        ),
        (
            "a zero-match multi-token",
            "'zzqx' & 'qqzx' & 'xzzq'",
            Some("('qqzx' & 'xzzq') | ('zzqx' & 'xzzq') | ('zzqx' & 'qqzx')"),
            "zzqx qqzx xzzq",
            None,
            "tracks_search_doc_gin",
        ),
        (
            "an owner scope",
            "'love'",
            None,
            "love",
            Some("17"),
            "tracks_uploader_popular_idx",
        ),
    ] {
        let plan = explain!(
            tx,
            "../../../queries/search/tracks.sql",
            strict,
            loose,
            q,
            owner,
            21_i64,
            0_i64,
            false
        );
        assert_bounded(&plan, what);
        assert!(plan.contains(index), "{what}: {plan}");
    }

    let plan = explain!(
        tx,
        "../../../queries/search/playlists.sql",
        "'mix'",
        None::<String>,
        "mix",
        None::<String>,
        21_i64,
        0_i64
    );
    assert_bounded(&plan, "playlists");
    assert!(plan.contains("playlists_search_doc_gin"), "{plan}");

    let plan = explain!(
        tx,
        "../../../queries/search/users.sql",
        "'user42'",
        None::<String>,
        "user42",
        21_i64,
        0_i64
    );
    assert_bounded(&plan, "users");
    assert!(plan.contains("users_search_doc_gin"), "{plan}");

    let plan = explain!(
        tx,
        "../../../queries/search/artists.sql",
        "'artist42'",
        None::<String>,
        "artist42",
        21_i64,
        0_i64
    );
    assert!(plan.contains("artists_search_doc_gin"), "{plan}");

    let plan = explain!(
        tx,
        "../../../queries/search/albums.sql",
        "'record'",
        None::<String>,
        "record",
        21_i64,
        0_i64
    );
    assert!(plan.contains("albums_search_doc_gin"), "{plan}");

    let plan = explain!(
        tx,
        "../../../queries/search/lyrics.sql",
        "'forever' & 'heart'",
        Some("'forever' | 'heart'"),
        "forever heart",
        21_i64,
        0_i64
    );
    assert_bounded(&plan, "lyrics");
    assert!(plan.contains("lyrics_cache_fts_col_gin"), "{plan}");

    let plan = explain!(
        tx,
        "../../../queries/search/terms.sql",
        "forevr hear",
        8_i64
    );
    assert!(plan.contains("search_terms_latin"), "{plan}");
    assert!(
        !seq_scans(&plan).contains(&"search_terms".to_owned()),
        "{plan}"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn short_unknown_tokens_skip_the_fuzzy_arm(pool: PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, metadata_artist, uploader_username,
                             uploader_sc_user_id, duration_ms, play_count_sc, sharing)
         SELECT n::text, 'soundcloud:tracks:' || n, 'love song', 'love song', 'band', 'band', '1', 1000, n, 'public'
         FROM generate_series(1, 3) n;
         REFRESH MATERIALIZED VIEW search_terms;",
    )
    .execute(&pool)
    .await?;
    let mut tx = configured(&pool).await?;
    let rows = sqlx::query_file_as!(
        TermRow,
        "queries/search/terms.sql",
        "xx lo zq yv lov band",
        8_i64
    )
    .fetch_all(&mut *tx)
    .await?;
    let near: Vec<&str> = rows
        .iter()
        .filter(|row| row.kind.as_deref() == Some("near"))
        .map(|row| row.lexeme.as_str())
        .collect();
    assert_eq!(near, ["lov"]);
    Ok(())
}

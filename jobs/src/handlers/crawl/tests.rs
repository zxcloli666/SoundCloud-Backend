use sqlx::PgPool;

use super::*;

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE artists (
             id uuid PRIMARY KEY,
             name text NOT NULL,
             mb_artist_id text,
             genius_artist_id text,
             sc_user_id text,
             has_sc_account boolean NOT NULL DEFAULT false,
             merged_into uuid,
             mb_crawl_offset integer NOT NULL DEFAULT 0,
             genius_crawl_offset integer NOT NULL DEFAULT 0,
             mb_crawled_at timestamptz,
             genius_crawled_at timestamptz,
             mb_next_run_at timestamptz NOT NULL DEFAULT now(),
             genius_next_run_at timestamptz NOT NULL DEFAULT now(),
             mb_locked_at timestamptz,
             genius_locked_at timestamptz,
             crawl_fail_count smallint NOT NULL DEFAULT 0,
             crawl_dead boolean NOT NULL DEFAULT false,
             last_crawled_at timestamptz,
             crawl_attempts smallint NOT NULL DEFAULT 0,
             updated_at timestamptz NOT NULL DEFAULT now()
         );",
    )
    .execute(pool)
    .await?;
    Ok(())
}

struct Seed {
    name: &'static str,
    mb_artist_id: Option<&'static str>,
    genius_artist_id: Option<&'static str>,
    has_sc_account: bool,
}

impl Seed {
    fn artist(name: &'static str) -> Self {
        Self {
            name,
            mb_artist_id: None,
            genius_artist_id: None,
            has_sc_account: false,
        }
    }

    fn on_genius(mut self, id: &'static str) -> Self {
        self.genius_artist_id = Some(id);
        self
    }

    fn on_musicbrainz(mut self, id: &'static str) -> Self {
        self.mb_artist_id = Some(id);
        self
    }

    fn verified_on_soundcloud(mut self) -> Self {
        self.has_sc_account = true;
        self
    }
}

async fn seed(pool: &PgPool, seed: Seed) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO artists (id, name, mb_artist_id, genius_artist_id, has_sc_account)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(seed.name)
    .bind(seed.mb_artist_id)
    .bind(seed.genius_artist_id)
    .bind(seed.has_sc_account)
    .execute(pool)
    .await?;
    Ok(id)
}

async fn claim_identity(pool: &PgPool) -> anyhow::Result<Vec<Uuid>> {
    let rows = sqlx::query_file!("queries/crawl/claim_identity_lane.sql", 600.0, 10)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|row| row.id).collect())
}

async fn claim_genius(pool: &PgPool) -> anyhow::Result<Vec<Uuid>> {
    let rows = sqlx::query_file!("queries/crawl/claim_genius_lane.sql", 600.0, 10)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|row| row.id).collect())
}

#[sqlx::test(migrations = false)]
async fn a_verified_artist_without_a_genius_id_is_looked_up(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let verified = seed(&pool, Seed::artist("Мокери").verified_on_soundcloud()).await?;
    seed(&pool, Seed::artist("Unknown")).await?;
    seed(
        &pool,
        Seed::artist("Already Known")
            .verified_on_soundcloud()
            .on_genius("42"),
    )
    .await?;

    let claimed = claim_identity(&pool).await?;

    assert_eq!(claimed, vec![verified]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_matched_identity_becomes_due_for_the_genius_lane(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed(&pool, Seed::artist("Мокери").verified_on_soundcloud()).await?;
    claim_identity(&pool).await?;

    sqlx::query_file!("queries/crawl/attach_genius_artist_id.sql", artist, "1312")
        .execute(&pool)
        .await?;

    assert_eq!(claim_genius(&pool).await?, vec![artist]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unmatched_identity_is_retried_later_not_forgotten(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed(&pool, Seed::artist("Мокери").verified_on_soundcloud()).await?;
    claim_identity(&pool).await?;

    sqlx::query_file!("queries/crawl/defer_identity_lookup.sql", artist, 14.0)
        .execute(&pool)
        .await?;

    assert!(claim_identity(&pool).await?.is_empty());
    let due_within_a_month: bool = sqlx::query_scalar(
        "SELECT genius_next_run_at BETWEEN now() AND now() + interval '30 days'
         FROM artists WHERE id = $1",
    )
    .bind(artist)
    .fetch_one(&pool)
    .await?;
    assert!(due_within_a_month);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_musicbrainz_lane_leaves_genius_artists_to_the_genius_lane(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let mb_only = seed(&pool, Seed::artist("MB Only").on_musicbrainz("mbid-1")).await?;
    seed(
        &pool,
        Seed::artist("Both").on_musicbrainz("mbid-2").on_genius("7"),
    )
    .await?;

    let rows = sqlx::query_file!("queries/crawl/claim_mb_lane.sql", 600.0, 10)
        .fetch_all(&pool)
        .await?;

    assert_eq!(
        rows.into_iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![mb_only]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_failing_artist_backs_off_before_it_is_declared_dead(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed(&pool, Seed::artist("Мокери").on_genius("1312")).await?;
    claim_genius(&pool).await?;

    sqlx::query_file!(
        "queries/crawl/lane_backoff_genius.sql",
        artist,
        3i16,
        next_run_after(3)
    )
    .execute(&pool)
    .await?;

    assert!(claim_genius(&pool).await?.is_empty());
    let dead: bool = sqlx::query_scalar("SELECT crawl_dead FROM artists WHERE id = $1")
        .bind(artist)
        .fetch_one(&pool)
        .await?;
    assert!(!dead);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_dead_artist_is_never_claimed_again(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed(&pool, Seed::artist("Мокери").on_genius("1312")).await?;
    claim_genius(&pool).await?;

    sqlx::query_file!("queries/crawl/lane_dead_genius.sql", artist, 8i16)
        .execute(&pool)
        .await?;

    assert!(claim_genius(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_successful_genius_crawl_refreshes_both_cursors(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed(
        &pool,
        Seed::artist("Мокери")
            .on_genius("1312")
            .on_musicbrainz("mbid-1"),
    )
    .await?;
    claim_genius(&pool).await?;

    sqlx::query_file!("queries/crawl/lane_success_genius.sql", artist, 14.0)
        .execute(&pool)
        .await?;

    let (genius_crawled, mb_crawled): (bool, bool) = sqlx::query_as(
        "SELECT genius_crawled_at IS NOT NULL, mb_crawled_at IS NOT NULL
         FROM artists WHERE id = $1",
    )
    .bind(artist)
    .fetch_one(&pool)
    .await?;

    assert!(genius_crawled);
    assert!(mb_crawled);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_crawled_artist_with_an_unknown_id_is_not_a_short_named_local_one(
    pool: PgPool,
) -> anyhow::Result<()> {
    let z2: Uuid = sqlx::query_scalar(
        "INSERT INTO artists (name, normalized_name, source) VALUES ('z²', 'z2', 'mb') RETURNING id",
    )
    .fetch_one(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO artist_sc_accounts (artist_id, sc_user_id, role, source, verified)
         VALUES ($1, '1211580634', 'main', 'mb_resolve', false)",
    )
    .bind(z2)
    .execute(&pool)
    .await?;
    let oyg: Uuid = sqlx::query_scalar(
        "INSERT INTO artists (name, normalized_name, genius_artist_id, source)
         VALUES ('oyg', 'oyg', '111', 'genius') RETURNING id",
    )
    .fetch_one(&pool)
    .await?;
    let monarchy: Uuid = sqlx::query_scalar(
        "INSERT INTO artists (name, normalized_name, source) VALUES ('Monarchy', 'monarchy', 'mb') RETURNING id",
    )
    .fetch_one(&pool)
    .await?;

    assert_eq!(
        entities::artist_id(&pool, None, Some("222"), "Z2").await?,
        None
    );
    assert_eq!(
        entities::artist_id(&pool, None, Some("333"), "OYG").await?,
        None
    );
    assert_eq!(
        entities::artist_id(&pool, None, Some("444"), "MONARCHY").await?,
        Some(monarchy)
    );

    let genius_ids: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT genius_artist_id FROM artists WHERE id = ANY($1) ORDER BY normalized_name",
    )
    .bind([monarchy, oyg, z2])
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        genius_ids,
        [Some("444".to_owned()), Some("111".to_owned()), None]
    );
    Ok(())
}

async fn seed_artist(pool: &PgPool, name: &str) -> anyhow::Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO artists (name, normalized_name, source) VALUES ($1, lower($1), 'genius') RETURNING id",
    )
    .bind(name)
    .fetch_one(pool)
    .await?)
}

fn draft(external_id: &str, primary: Uuid, aliases: &[&str]) -> page::WantedDraft {
    let mut forms = catalog_normalize::title_forms(external_id);
    forms.aliases = aliases.iter().map(|alias| (*alias).to_owned()).collect();
    page::WantedDraft {
        title: format!("Song {external_id}"),
        normalized: format!("song {external_id}"),
        primary_artist_id: Some(primary),
        external_id: external_id.to_owned(),
        forms,
        featured: Vec::new(),
        album: None,
    }
}

async fn aliases_of(pool: &PgPool, external_id: &str) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT alias.alias_key
         FROM wanted_track_work_aliases AS alias
         JOIN wanted_tracks AS wanted ON wanted.id = alias.wanted_track_id
         WHERE wanted.external_id = $1
         ORDER BY alias.alias_key",
    )
    .bind(external_id)
    .fetch_all(pool)
    .await?)
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_flushed_page_writes_the_wanted_rows_with_credits_aliases_and_albums(
    pool: PgPool,
) -> anyhow::Result<()> {
    let primary = seed_artist(&pool, "Мокери").await?;
    let featured = seed_artist(&pool, "Gost").await?;
    let album: Uuid = sqlx::query_scalar(
        "INSERT INTO albums (title, normalized_title, source) VALUES ('Al', 'al', 'genius') RETURNING id",
    )
    .fetch_one(&pool)
    .await?;
    let mut page = page::Page::default();
    let mut first = draft("1", primary, &["a", "b"]);
    first.featured = vec![(featured, 0)];
    first.album = Some((album, 3));
    page.add(&pool, first).await?;
    page.add(&pool, draft("2", primary, &[])).await?;
    page.flush(&pool).await?;

    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM wanted_tracks WHERE source = 'genius_crawl'")
            .fetch_one(&pool)
            .await?;
    let credits: Vec<(String, String, i16)> = sqlx::query_as(
        "SELECT credit.role, wanted.external_id, credit.position
         FROM wanted_track_artists AS credit
         JOIN wanted_tracks AS wanted ON wanted.id = credit.wanted_track_id
         ORDER BY wanted.external_id, credit.role",
    )
    .fetch_all(&pool)
    .await?;
    let placed: Vec<i16> = sqlx::query_scalar("SELECT position FROM wanted_track_albums")
        .fetch_all(&pool)
        .await?;

    assert_eq!(rows, 2);
    assert_eq!(
        credits,
        [
            ("featured".to_owned(), "1".to_owned(), 0),
            ("primary".to_owned(), "1".to_owned(), 0),
            ("primary".to_owned(), "2".to_owned(), 0),
        ]
    );
    assert_eq!(placed, [3]);
    assert_eq!(aliases_of(&pool, "1").await?, ["a", "b"]);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_recrawl_keeps_the_aliases_and_replaces_only_the_changed_ones(
    pool: PgPool,
) -> anyhow::Result<()> {
    let primary = seed_artist(&pool, "Мокери").await?;
    let mut page = page::Page::default();
    page.add(&pool, draft("1", primary, &["a", "b"])).await?;
    page.flush(&pool).await?;
    page.add(&pool, draft("1", primary, &["a", "b"])).await?;
    page.flush(&pool).await?;
    let unchanged = aliases_of(&pool, "1").await?;

    page.add(&pool, draft("1", primary, &["b", "c"])).await?;
    page.flush(&pool).await?;

    assert_eq!(unchanged, ["a", "b"]);
    assert_eq!(aliases_of(&pool, "1").await?, ["b", "c"]);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn the_single_row_alias_store_keeps_aliases_on_a_repeat(pool: PgPool) -> anyhow::Result<()> {
    let primary = seed_artist(&pool, "Мокери").await?;
    let mut page = page::Page::default();
    page.add(&pool, draft("1", primary, &[])).await?;
    page.flush(&pool).await?;
    let wanted_id: Uuid =
        sqlx::query_scalar("SELECT id FROM wanted_tracks WHERE external_id = '1'")
            .fetch_one(&pool)
            .await?;
    let set = |aliases: &[&str]| aliases.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>();

    entities::store_wanted_aliases(&pool, wanted_id, &set(&["a", "b"])).await?;
    entities::store_wanted_aliases(&pool, wanted_id, &set(&["a", "b"])).await?;
    let repeated = aliases_of(&pool, "1").await?;
    entities::store_wanted_aliases(&pool, wanted_id, &set(&["b", "c"])).await?;
    let changed = aliases_of(&pool, "1").await?;
    entities::store_wanted_aliases(&pool, wanted_id, &[]).await?;

    assert_eq!(repeated, ["a", "b"]);
    assert_eq!(changed, ["b", "c"]);
    assert!(aliases_of(&pool, "1").await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn an_unchanged_recrawl_does_not_touch_the_wanted_row(pool: PgPool) -> anyhow::Result<()> {
    let primary = seed_artist(&pool, "Мокери").await?;
    let mut page = page::Page::default();
    page.add(&pool, draft("1", primary, &[])).await?;
    page.flush(&pool).await?;
    let before: i64 =
        sqlx::query_scalar("SELECT xmin::text::bigint FROM wanted_tracks WHERE external_id = '1'")
            .fetch_one(&pool)
            .await?;

    page.add(&pool, draft("1", primary, &[])).await?;
    page.flush(&pool).await?;
    let after: i64 =
        sqlx::query_scalar("SELECT xmin::text::bigint FROM wanted_tracks WHERE external_id = '1'")
            .fetch_one(&pool)
            .await?;

    assert_eq!(before, after);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn the_same_song_twice_in_one_page_is_written_in_order(pool: PgPool) -> anyhow::Result<()> {
    let primary = seed_artist(&pool, "Мокери").await?;
    let mut page = page::Page::default();
    page.add(&pool, draft("1", primary, &["a"])).await?;
    page.add(&pool, draft("1", primary, &["b"])).await?;
    page.flush(&pool).await?;

    assert_eq!(aliases_of(&pool, "1").await?, ["b"]);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_page_resolves_the_same_artist_once(pool: PgPool) -> anyhow::Result<()> {
    let artist = seed_artist(&pool, "Gost").await?;
    let mut page = page::Page::default();

    let first = page.artist_id(&pool, Some("77"), "Gost").await?;
    sqlx::query("DELETE FROM artists WHERE id = $1")
        .bind(artist)
        .execute(&pool)
        .await?;
    let second = page.artist_id(&pool, Some("77"), "Gost").await?;

    assert_eq!(first, Some(artist));
    assert_eq!(second, first);
    Ok(())
}

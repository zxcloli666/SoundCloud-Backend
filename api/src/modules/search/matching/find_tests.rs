use sqlx::PgPool;

use super::find::find;
use super::wanted::Wanted;
use crate::modules::search::catalog_tests::{Song, seed, service, song};

fn long(mut song: Song, duration_ms: i32) -> Song {
    song.duration_ms = duration_ms;
    song
}

fn by(mut song: Song, uploader: &str) -> Song {
    song.uploader = uploader.to_owned();
    song
}

fn urn(id: i64) -> Option<String> {
    Some(format!("soundcloud:tracks:{id}"))
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_close_duration_picks_the_right_copy(pool: PgPool) -> anyhow::Result<()> {
    seed(
        &pool,
        &[
            long(song(1, "Halo", "Beyonce", 90_000_000), 1_800_000),
            long(song(2, "Halo", "Beyonce", 1_000), 261_000),
        ],
    )
    .await?;
    let search = service(&pool)?;
    let wanted = Wanted::new("Beyonce", "Halo", Some(261_000));
    assert_eq!(find(&search, &wanted).await?, urn(2));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_hit_with_another_title_is_a_miss(pool: PgPool) -> anyhow::Result<()> {
    seed(
        &pool,
        &[
            song(1, "Nothing Else Matters", "Metallica", 9_000_000),
            song(2, "Enter Sandman", "Metallica", 8_000_000),
        ],
    )
    .await?;
    let search = service(&pool)?;
    let wanted = Wanted::new("Metallica", "Master of Puppets", Some(200_000));
    assert_eq!(find(&search, &wanted).await?, None);
    let wanted = Wanted::new("Metallica", "Enter Sandman (Official Video)", Some(200_000));
    assert_eq!(find(&search, &wanted).await?, urn(2));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_artists_own_upload_beats_a_more_popular_reupload(pool: PgPool) -> anyhow::Result<()> {
    seed(
        &pool,
        &[
            by(song(1, "Hello", "Adele", 50_000_000), "random reposter"),
            long(by(song(2, "Hello", "Adele", 1_000), "Adele"), 206_000),
        ],
    )
    .await?;
    let search = service(&pool)?;
    let wanted = Wanted::new("Adele", "Hello", Some(200_000));
    assert_eq!(find(&search, &wanted).await?, urn(2));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_title_search_finds_the_artists_upload_the_combined_one_missed(
    pool: PgPool,
) -> anyhow::Result<()> {
    let mut songs: Vec<Song> = (1..=6)
        .map(|n| long(song(n, "Zed Moon", "Zed Moon", 80_000_000), 600_000))
        .collect();
    songs.push(by(song(7, "Moon", "Zed", 10), "Zed"));
    seed(&pool, &songs).await?;
    let search = service(&pool)?;
    let wanted = Wanted::new("Zed", "Moon", Some(200_000));
    assert_eq!(find(&search, &wanted).await?, urn(7));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn an_empty_title_or_unknown_song_is_a_miss(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool, &[song(1, "Umbrella", "Rihanna", 1_000)]).await?;
    let search = service(&pool)?;
    for (artist, title) in [
        ("Rihanna", ""),
        ("Rihanna", "!!!"),
        ("", ""),
        ("Nobody", "Nothing at all"),
    ] {
        let wanted = Wanted::new(artist, title, None);
        assert_eq!(find(&search, &wanted).await?, None, "{artist} / {title}");
    }
    Ok(())
}

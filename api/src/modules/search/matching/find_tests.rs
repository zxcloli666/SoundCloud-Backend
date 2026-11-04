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

#[sqlx::test(migrations = "./migrations")]
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

#[sqlx::test(migrations = "./migrations")]
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

#[sqlx::test(migrations = "./migrations")]
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

#[sqlx::test(migrations = "./migrations")]
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

#[sqlx::test(migrations = "./migrations")]
async fn a_label_upload_beats_reuploads_and_remixes(pool: PgPool) -> anyhow::Result<()> {
    let reupload = |id, title: &str, uploader: &str, plays, ms| {
        long(by(song(id, title, "", plays), uploader), ms)
    };
    seed(
        &pool,
        &[
            long(
                by(
                    song(
                        2401939677,
                        "RHYNO (prod. by Guy-Manuel de Homem-Christo) (from GTAVI: The Album)",
                        "Travis Scott",
                        78_517,
                    ),
                    "Atlantic Records",
                ),
                180_000,
            ),
            reupload(
                2402259756,
                "Travis Scott – RHYNO (GTA VI)",
                "ay4z",
                11_968,
                239_698,
            ),
            reupload(
                2418627087,
                "Travis Scott RHYNO GTA VI (Madtrix Remix)",
                "Motta",
                145,
                188_800,
            ),
            reupload(
                2405017446,
                "Travis Scott - RHYNO (JYTTRO EDIT) - FREE DL",
                "JYTTRO",
                129,
                185_316,
            ),
            reupload(
                2403358584,
                "Travis Scott - RHYNO (Remix) (GTA VI Soundtrack)",
                "JaRoNiMo BeaTZ",
                77,
                174_552,
            ),
        ],
    )
    .await?;
    let search = service(&pool)?;
    let wanted = Wanted::new(
        "Travis Scott, Grand Theft Auto VI",
        "RHYNO (from GTAVI: The Album)",
        Some(180_000),
    );
    assert_eq!(find(&search, &wanted).await?, urn(2401939677));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
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

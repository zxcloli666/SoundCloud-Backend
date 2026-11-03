use super::wanted::{Wanted, clean_title, first_artist, simplify};

#[test]
fn only_the_first_artist_is_kept() {
    for (raw, expected) in [
        ("Metallica", "Metallica"),
        ("Skriptonit, Niman", "Skriptonit"),
        ("Rihanna & Calvin Harris", "Rihanna"),
        ("Dua Lipa feat. Elton John", "Dua Lipa"),
        ("Dua Lipa ft. Elton John", "Dua Lipa"),
        ("Mumford x Sons", "Mumford"),
        ("Boris Brejcha - Topic", "Boris Brejcha"),
        ("AdeleVEVO", "Adele"),
        ("  Lizer  ", "Lizer"),
    ] {
        assert_eq!(first_artist(raw), expected, "{raw}");
    }
}

#[test]
fn bracketed_noise_is_dropped_from_titles() {
    for (raw, expected) in [
        ("Umbrella (Official Video)", "Umbrella"),
        ("Umbrella [Official Audio]", "Umbrella"),
        ("Положение (Премьера клипа)", "Положение"),
        ("Halo (Lyrics)", "Halo"),
        (
            "Nothing Else Matters (Remastered 2021)",
            "Nothing Else Matters",
        ),
        ("Waterloo (feat. Someone)", "Waterloo"),
        ("Song Name | Official Channel", "Song Name"),
        ("Song - 2011 Remaster", "Song"),
        ("Song feat. Guest", "Song"),
        ("Song (Live at Wembley)", "Song (Live at Wembley)"),
        ("(Official Video)", "(Official Video)"),
    ] {
        assert_eq!(clean_title(raw), expected, "{raw}");
    }
}

#[test]
fn comparison_keeps_letters_and_digits_only() {
    assert_eq!(simplify("Beyoncé - Halo!"), "beyoncéhalo");
    assert_eq!(simplify("Кукла колдуна"), "куклаколдуна");
}

#[test]
fn identical_normalized_items_share_a_key() {
    let a = Wanted::new("Rihanna", "Umbrella (Official Video)", Some(200_000));
    let b = Wanted::new("rihanna feat. Jay-Z", "UMBRELLA", Some(200_000));
    let c = Wanted::new("Rihanna", "Umbrella", Some(300_000));
    assert_eq!(a.key(), b.key());
    assert_ne!(a.key(), c.key());
}

#[test]
fn a_missing_or_zero_duration_is_unknown() {
    assert_eq!(Wanted::new("a", "b", Some(0)).duration_ms, None);
    assert_eq!(Wanted::new("a", "b", None).duration_ms, None);
    assert_eq!(Wanted::new("a", "b", Some(1000)).duration_ms, Some(1000));
}

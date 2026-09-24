pub fn canonicalize_tags(raw: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::with_capacity(raw.len());
    raw.into_iter()
        .filter_map(|t| canonicalize_tag(&t))
        .filter(|tag| seen.insert(tag.clone()))
        .collect()
}

pub fn canonicalize_tag(raw: &str) -> Option<String> {
    let lower = raw.trim().to_lowercase();
    if lower.is_empty() {
        return None;
    }
    let canon = match lower.as_str() {
        "hip hop" | "hip-hop" | "hiphop" | "rap" => "Hip-Hop",
        "r&b" | "rnb" | "r and b" | "rhythm and blues" => "R&B",
        "drum and bass" | "drum & bass" | "dnb" | "drumandbass" => "Drum & Bass",
        "electronic" | "electronica" | "edm" => "Electronic",
        "synthwave" | "synth-wave" | "synth wave" => "Synthwave",
        "lo-fi" | "lofi" | "lo fi" => "Lofi",
        "indie" => "Indie",
        "indie pop" => "Indie Pop",
        "indie rock" => "Indie Rock",
        "pop" => "Pop",
        "rock" => "Rock",
        "house" => "House",
        "deep house" => "Deep House",
        "techno" => "Techno",
        "minimal" => "Minimal",
        "ambient" => "Ambient",
        "trap" => "Trap",
        "jazz" => "Jazz",
        "soul" => "Soul",
        "funk" => "Funk",
        "experimental" => "Experimental",
        "shoegaze" => "Shoegaze",
        "post-rock" | "post rock" | "postrock" => "Post-Rock",
        "garage" => "Garage",
        "punk" => "Punk",
        "afrobeat" => "Afrobeat",
        "latin" => "Latin",
        "bossa" | "bossa nova" => "Bossa Nova",
        "cinematic" => "Cinematic",
        "neoclassical" | "neo-classical" => "Neoclassical",
        "industrial" => "Industrial",
        "drone" => "Drone",
        "chillwave" => "Chillwave",
        "surf" => "Surf",
        "idm" => "IDM",
        "dream pop" | "dreampop" => "Dream Pop",
        "synthpop" | "synth-pop" | "synth pop" => "Synthpop",
        "new wave" | "newwave" => "New Wave",
        "bedroom" => "Bedroom",
        "folk" => "Folk",
        "acoustic" => "Acoustic",
        "singer/songwriter" | "singer-songwriter" => "Singer/Songwriter",
        _ => return Some(title_case(&lower)),
    };
    Some(canon.to_string())
}

fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = true;
    for c in s.chars() {
        if c.is_whitespace() || c == '-' {
            out.push(c);
            prev_space = true;
        } else if prev_space {
            for u in c.to_uppercase() {
                out.push(u);
            }
            prev_space = false;
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spelling_of_a_genre_lands_on_one_label() {
        for raw in ["rap", "RAP", " Hip Hop ", "hip-hop", "HIPHOP"] {
            assert_eq!(
                canonicalize_tag(raw).as_deref(),
                Some("Hip-Hop"),
                "{raw} must canonicalize"
            );
        }
        for raw in ["dnb", "Drum And Bass", "drum & bass"] {
            assert_eq!(canonicalize_tag(raw).as_deref(), Some("Drum & Bass"));
        }
    }

    #[test]
    fn an_unknown_tag_is_title_cased_and_keeps_its_separators() {
        assert_eq!(
            canonicalize_tag("dark  jazzy-beats").as_deref(),
            Some("Dark  Jazzy-Beats")
        );
        assert_eq!(canonicalize_tag("ГОРОДСКОЙ").as_deref(), Some("Городской"));
    }

    #[test]
    fn a_tag_of_nothing_is_dropped() {
        assert_eq!(canonicalize_tag(""), None);
        assert_eq!(canonicalize_tag("   "), None);
        assert_eq!(
            canonicalize_tags(vec!["".into(), " ".into(), "pop".into()]),
            vec!["Pop".to_owned()]
        );
    }

    #[test]
    fn two_spellings_of_one_genre_do_not_become_two_tags() {
        assert_eq!(
            canonicalize_tags(vec!["rap".into(), "hip hop".into(), "Trap".into()]),
            vec!["Hip-Hop".to_owned(), "Trap".to_owned()],
            "a track tagged twice with the same genre must not show it twice"
        );
    }

    #[test]
    fn the_order_the_uploader_chose_is_kept() {
        assert_eq!(
            canonicalize_tags(vec!["techno".into(), "ambient".into(), "house".into()]),
            vec![
                "Techno".to_owned(),
                "Ambient".to_owned(),
                "House".to_owned()
            ]
        );
    }
}

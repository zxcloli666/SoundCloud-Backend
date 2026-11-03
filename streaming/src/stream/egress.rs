use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(crate) const TAG_HEADER: &str = "x-egress-tag";
const API_V2_HOST: &str = "api-v2.soundcloud.com";
const REGION_KEPT_FOR: Duration = Duration::from_secs(24 * 60 * 60);
const NOWHERE_KEPT_FOR: Duration = Duration::from_secs(60 * 60);
const SWEEP_EVERY: Duration = Duration::from_secs(60);
const TRACKS_AT_MOST: usize = 50_000;

pub struct Egress {
    regions: Vec<String>,
    seen: Mutex<Seen>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Known {
    Unknown,
    Region(String),
    Nowhere,
}

#[derive(Default)]
struct Seen {
    tracks: HashMap<String, (Instant, Option<usize>)>,
    swept: Option<Instant>,
}

impl Egress {
    pub fn new(regions: Vec<String>) -> Self {
        Self {
            regions,
            seen: Mutex::default(),
        }
    }

    pub(crate) fn regions(&self) -> &[String] {
        &self.regions
    }

    pub(crate) fn known(&self, track_id: &str) -> Known {
        self.known_at(track_id, Instant::now())
    }

    pub(crate) fn remember(&self, track_id: &str, region: &str) {
        if let Some(index) = self.regions.iter().position(|r| r == region) {
            self.note_at(track_id, Some(index), Instant::now());
        }
    }

    pub(crate) fn remember_nowhere(&self, track_id: &str) {
        self.note_at(track_id, None, Instant::now());
    }

    pub(crate) fn forget(&self, track_id: &str) {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.tracks.remove(track_id);
    }

    fn known_at(&self, track_id: &str, now: Instant) -> Known {
        let seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        match seen.tracks.get(track_id) {
            Some((at, region)) if now.duration_since(*at) < kept_for(*region) => match region {
                Some(index) => self
                    .regions
                    .get(*index)
                    .map_or(Known::Unknown, |region| Known::Region(region.clone())),
                None => Known::Nowhere,
            },
            _ => Known::Unknown,
        }
    }

    fn note_at(&self, track_id: &str, region: Option<usize>, now: Instant) {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let sweep_due = seen
            .swept
            .is_none_or(|swept| now.duration_since(swept) >= SWEEP_EVERY);
        if seen.tracks.len() >= TRACKS_AT_MOST && sweep_due {
            seen.tracks
                .retain(|_, (at, region)| now.duration_since(*at) < kept_for(*region));
            seen.swept = Some(now);
        }
        if seen.tracks.len() < TRACKS_AT_MOST || seen.tracks.contains_key(track_id) {
            seen.tracks.insert(track_id.to_owned(), (now, region));
        }
    }
}

fn kept_for(region: Option<usize>) -> Duration {
    match region {
        Some(_) => REGION_KEPT_FOR,
        None => NOWHERE_KEPT_FOR,
    }
}

pub(crate) fn parse_regions(raw: &str) -> Vec<String> {
    let mut regions: Vec<String> = Vec::new();
    for tag in raw.split(',').map(str::trim) {
        let tag = tag.to_ascii_lowercase();
        let readable = !tag.is_empty()
            && tag
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if readable && !regions.contains(&tag) {
            regions.push(tag);
        }
    }
    regions
}

pub(crate) fn steered(region: Option<&str>) -> HashMap<String, String> {
    region
        .map(|region| HashMap::from([(TAG_HEADER.to_owned(), region.to_owned())]))
        .unwrap_or_default()
}

pub(crate) fn keeps_tag(
    headers: &mut HashMap<String, String>,
    target_url: &str,
    proxy_set: bool,
) -> bool {
    if !headers.contains_key(TAG_HEADER) {
        return false;
    }
    let api_v2 = url::Url::parse(target_url)
        .ok()
        .is_some_and(|url| url.host_str() == Some(API_V2_HOST));
    if proxy_set && api_v2 {
        return true;
    }
    headers.remove(TAG_HEADER);
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACK: &str = "1696778178";

    fn egress() -> Egress {
        Egress::new(parse_regions("geo-us,geo-fr"))
    }

    #[test]
    fn a_region_that_answered_is_tried_first_for_a_day() {
        let egress = egress();
        let now = Instant::now();
        assert_eq!(egress.known_at(TRACK, now), Known::Unknown);

        egress.note_at(TRACK, Some(1), now);
        assert_eq!(
            egress.known_at(TRACK, now + REGION_KEPT_FOR / 2),
            Known::Region("geo-fr".into())
        );
        assert_eq!(
            egress.known_at(TRACK, now + REGION_KEPT_FOR),
            Known::Unknown,
            "proxies rotate every six days and soundcloud changes blocks; old knowledge goes"
        );
    }

    #[test]
    fn a_track_blocked_everywhere_is_left_alone_for_an_hour_only() {
        let egress = egress();
        let now = Instant::now();
        egress.note_at(TRACK, None, now);

        assert_eq!(egress.known_at(TRACK, now), Known::Nowhere);
        assert_eq!(
            egress.known_at(TRACK, now + NOWHERE_KEPT_FOR),
            Known::Unknown,
            "a region missing from the pool for a while must not hide the track for a day"
        );
    }

    #[test]
    fn only_a_configured_region_is_remembered() {
        let egress = egress();
        egress.remember(TRACK, "geo-ru");
        assert_eq!(egress.known(TRACK), Known::Unknown);

        egress.remember(TRACK, "geo-us");
        assert_eq!(egress.known(TRACK), Known::Region("geo-us".into()));
        egress.forget(TRACK);
        assert_eq!(egress.known(TRACK), Known::Unknown);
    }

    #[test]
    fn the_memory_stays_bounded() {
        let egress = egress();
        let now = Instant::now();
        for id in 0..TRACKS_AT_MOST + 10 {
            egress.note_at(&id.to_string(), Some(0), now);
        }
        assert_eq!(
            egress.seen.lock().unwrap().tracks.len(),
            TRACKS_AT_MOST,
            "a crawl over millions of tracks must not grow this map without end"
        );

        egress.note_at("0", None, now);
        assert_eq!(egress.known_at("0", now), Known::Nowhere);

        let later = now + REGION_KEPT_FOR + SWEEP_EVERY;
        egress.note_at("fresh", Some(1), later);
        assert_eq!(
            egress.known_at("fresh", later),
            Known::Region("geo-fr".into())
        );
    }

    #[test]
    fn regions_are_read_as_proxy_tags() {
        assert_eq!(
            parse_regions(" geo-US, geo-fr,,geo-fr, bad tag, geo_de "),
            vec!["geo-us", "geo-fr", "geo_de"]
        );
        assert!(parse_regions("").is_empty());
    }

    #[test]
    fn the_tag_steers_only_api_v2_behind_our_proxy() {
        let target = "https://api-v2.soundcloud.com/tracks/1?client_id=x";
        let mut headers = steered(Some("geo-us"));
        assert!(keeps_tag(&mut headers, target, true));
        assert_eq!(headers.get(TAG_HEADER).map(String::as_str), Some("geo-us"));

        for (url, proxy_set) in [
            (target, false),
            (
                "https://playback.media-streaming.soundcloud.cloud/cenc/x",
                true,
            ),
            (
                "https://license.media-streaming.soundcloud.cloud/playback/widevine",
                true,
            ),
            ("https://api.soundcloud.com/tracks/1", true),
            ("https://api-v2.soundcloud.com.evil.example/tracks/1", true),
        ] {
            let mut headers = steered(Some("geo-us"));
            assert!(!keeps_tag(&mut headers, url, proxy_set));
            assert!(
                headers.is_empty(),
                "{url}: only the api-v2 route reads the tag and drops it; anywhere else it \
                 would reach soundcloud as a header no browser sends"
            );
        }

        let mut plain = HashMap::new();
        assert!(!keeps_tag(&mut plain, target, true));
        assert!(steered(None).is_empty());
    }
}

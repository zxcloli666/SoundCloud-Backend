use std::borrow::Cow;

use wreq::http2::{Http2Options, StreamDependency, StreamId};
use wreq::tls::KeyShare;

const FIREFOX_LEGACY_KEY_SHARES: &[KeyShare] = &[KeyShare::X25519, KeyShare::P256];

pub(crate) fn restore(profile: &str, emulation: &mut wreq::Emulation) {
    if matches!(profile, "firefox_109" | "firefox_117") {
        if let Some(tls) = emulation.tls_options.as_mut() {
            tls.key_shares = Some(Cow::Borrowed(FIREFOX_LEGACY_KEY_SHARES));
        }
    }

    let Some(http2) = emulation.http2_options.as_mut() else {
        return;
    };

    if profile == "okhttp_5" {
        restore_okhttp5_settings(http2);
    }

    if let Some(priority) = headers_priority(profile, http2.headers_stream_dependency) {
        http2.headers_stream_dependency = Some(priority);
    }
}

fn restore_okhttp5_settings(http2: &mut Http2Options) {
    http2.header_table_size = Some(65536);
    http2.max_concurrent_streams = Some(1000);
    http2.initial_window_size = 6291456;
    http2.max_header_list_size = Some(262144);
    http2.initial_conn_window_size = 15728640;
}

fn headers_priority(profile: &str, current: Option<StreamDependency>) -> Option<StreamDependency> {
    let family = profile.split('_').next().unwrap_or(profile);
    let (weight, exclusive) = match (family, profile) {
        ("chrome" | "edge" | "opera", _) | (_, "safari_16" | "okhttp_5") => (255, true),
        (_, "safari_18") => (255, false),
        ("firefox", _) => (41, false),
        _ => return None,
    };
    let parent = current.map_or(StreamId::zero(), |dependency| dependency.dependency_id());
    Some(StreamDependency::new(parent, weight, exclusive))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn priority_of(profile: &str) -> Option<StreamDependency> {
        crate::emulation(Some(profile))
            .1
            .http2_options
            .and_then(|http2| http2.headers_stream_dependency)
    }

    #[test]
    fn chrome_headers_keep_the_highest_weight() {
        assert_eq!(
            priority_of("chrome_137"),
            Some(StreamDependency::new(StreamId::zero(), 255, true))
        );
        assert_eq!(
            priority_of("edge_134"),
            Some(StreamDependency::new(StreamId::zero(), 255, true))
        );
    }

    #[test]
    fn firefox_headers_keep_their_parent_stream() {
        assert_eq!(
            priority_of("firefox_139"),
            Some(StreamDependency::new(StreamId::zero(), 41, false))
        );
        assert_eq!(
            priority_of("firefox_109"),
            Some(StreamDependency::new(StreamId::from(13), 41, false))
        );
    }

    #[test]
    fn safari_and_okhttp_headers_carry_a_priority_again() {
        assert_eq!(
            priority_of("safari_16"),
            Some(StreamDependency::new(StreamId::zero(), 255, true))
        );
        assert_eq!(
            priority_of("safari_18"),
            Some(StreamDependency::new(StreamId::zero(), 255, false))
        );
        assert_eq!(
            priority_of("okhttp_5"),
            Some(StreamDependency::new(StreamId::zero(), 255, true))
        );
    }

    #[test]
    fn unknown_families_keep_the_upstream_priority() {
        assert_eq!(headers_priority("safari_26", None), None);
        assert_eq!(headers_priority("okhttp_4.12", None), None);
    }

    #[test]
    fn old_firefox_offers_two_key_shares() {
        for profile in ["firefox_109", "firefox_117"] {
            let tls = crate::emulation(Some(profile))
                .1
                .tls_options
                .expect("tls options");
            assert_eq!(
                tls.key_shares.as_deref(),
                Some(FIREFOX_LEGACY_KEY_SHARES),
                "{profile}"
            );
        }
    }

    #[test]
    fn okhttp5_keeps_the_old_settings() {
        let http2 = crate::emulation(Some("okhttp_5"))
            .1
            .http2_options
            .expect("http2 options");
        assert_eq!(http2.header_table_size, Some(65536));
        assert_eq!(http2.max_concurrent_streams, Some(1000));
        assert_eq!(http2.initial_window_size, 6291456);
        assert_eq!(http2.max_header_list_size, Some(262144));
        assert_eq!(http2.initial_conn_window_size, 15728640);
    }
}

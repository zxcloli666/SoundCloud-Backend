pub const SQ: &str = "sq";
pub const HQ: &str = "hq";
pub const UQ: &str = "uq";

const HQ_MIN_KBPS: f64 = 144.0;
const UQ_MIN_KBPS: f64 = 224.0;

pub fn normalize(claimed: Option<&str>) -> &'static str {
    match claimed.map(str::trim) {
        Some(UQ) => UQ,
        Some(HQ) => HQ,
        _ => SQ,
    }
}

pub fn kbps(bytes: u64, secs: f64) -> Option<f64> {
    (secs > 0.0 && bytes > 0).then(|| bytes as f64 * 8.0 / secs / 1000.0)
}

pub fn of_kbps(kbps: f64) -> &'static str {
    if kbps >= UQ_MIN_KBPS {
        UQ
    } else if kbps >= HQ_MIN_KBPS {
        HQ
    } else {
        SQ
    }
}

pub fn output_bitrate(quality: &str) -> &'static str {
    if quality == UQ { "256k" } else { "192k" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured(kb: u64, secs: f64) -> &'static str {
        of_kbps(kbps(kb * 1000 / 8, secs).unwrap())
    }

    #[test]
    fn soundcloud_sources_land_on_their_tier() {
        assert_eq!(measured(128 * 180, 180.0), SQ);
        assert_eq!(measured(70 * 180, 180.0), SQ);
        assert_eq!(measured(163 * 180, 180.0), HQ);
        assert_eq!(measured(158 * 180, 180.0), HQ);
        assert_eq!(measured(259 * 180, 180.0), UQ);
    }

    #[test]
    fn nothing_is_measured_without_a_length() {
        assert_eq!(kbps(1_000_000, 0.0), None);
        assert_eq!(kbps(0, 180.0), None);
    }

    #[test]
    fn an_unknown_claim_counts_as_sq() {
        assert_eq!(normalize(Some("uq")), UQ);
        assert_eq!(normalize(Some(" hq ")), HQ);
        assert_eq!(normalize(Some("lossless")), SQ);
        assert_eq!(normalize(None), SQ);
    }

    #[test]
    fn only_uq_keeps_its_bitrate_in_storage() {
        assert_eq!(output_bitrate(UQ), "256k");
        assert_eq!(output_bitrate(HQ), "192k");
        assert_eq!(output_bitrate(SQ), "192k");
    }
}

use super::restricted::Transcoding;

pub const SQ: &str = "sq";
pub const HQ: &str = "hq";
pub const UQ: &str = "uq";

pub fn of_transcoding(t: &Transcoding) -> &'static str {
    let preset = t.preset.as_deref().unwrap_or("");
    if t.quality.as_deref() == Some("hq") || preset.starts_with("aac_256") {
        UQ
    } else if preset == "aac_160k" {
        HQ
    } else {
        SQ
    }
}

pub fn parse(label: &str) -> Option<&'static str> {
    match label {
        UQ => Some(UQ),
        HQ => Some(HQ),
        SQ => Some(SQ),
        _ => None,
    }
}

pub fn rank(quality: &str) -> u8 {
    match quality {
        UQ => 2,
        HQ => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transcoding(preset: &str, quality: &str) -> Transcoding {
        serde_json::from_value(serde_json::json!({
            "url": "https://api-v2.soundcloud.com/media/soundcloud:tracks:1/x/stream/hls",
            "preset": preset,
            "quality": quality,
        }))
        .unwrap()
    }

    #[test]
    fn soundcloud_presets_land_on_our_tiers() {
        assert_eq!(of_transcoding(&transcoding("aac_256k", "hq")), UQ);
        assert_eq!(of_transcoding(&transcoding("aac_160k", "sq")), HQ);
        assert_eq!(of_transcoding(&transcoding("mp3_1_0", "sq")), SQ);
        assert_eq!(of_transcoding(&transcoding("opus_0_0", "sq")), SQ);
        assert_eq!(of_transcoding(&transcoding("abr_sq", "sq")), SQ);
        assert_eq!(of_transcoding(&transcoding("aac_96k", "lq")), SQ);
    }

    #[test]
    fn tiers_order_from_sq_to_uq() {
        assert!(rank(SQ) < rank(HQ));
        assert!(rank(HQ) < rank(UQ));
        assert_eq!(rank("anything"), rank(SQ));
    }
}

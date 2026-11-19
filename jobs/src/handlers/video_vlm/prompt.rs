use backend_contracts::pipeline::VideoLabel;
use serde_json::Value;

use super::queue::Held;

const STATIC: &str = "static - no real filmed footage: one still picture or a few photos, album art, a logo, a drawn cover, a fan edit of photos, a screen recording, or a lyric video (song words on screen over a background, even a moving one). Effects on a still are not footage: zoom, pan, shake, flashes, glow, rain, particles, grain, color shifts, a waveform.";

const MOTION: &str = "clip - real filmed footage or real animation, however short and even if it repeats: people or the camera move, shots change. A phone video of a person or a concert is a clip even if the framing barely changes.";

const RULES: &str = "Rules: lyrics written over the picture means static. The same background in every frame with only the text changing is static.";

const JSON_REPLY: &str =
    r#"Reply as JSON: {"reason": "<one short sentence>", "label": "static|clip"}"#;

const LOCAL_INTRO: &str =
    "These images are frames from one music video, in time order, spread evenly over the video.";

const LOCAL_STATIC: &str = "static - no real filmed footage. One still picture or a few photos, album art, a logo, a drawn cover, a fan edit of photos, or a lyric video (song words on screen over a background). Effects on a still are not footage: zoom, pan, shake, flashes, glow, rain, particles, grain, color shifts, a moving waveform.";

const LOCAL_CLIP: &str = "clip - real filmed footage or real animation that goes on and changes over time: people move, the camera moves, shots and scenes change. A phone video of a person or a concert is a clip, even if the framing barely changes.";

const LOCAL_RULES: &str = "If song lyrics are written over the picture, it is static.";

const LOCAL_REPLY: &str = "Answer with one word: static or clip.";

const LOOP_MAX_S: f64 = 30.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Features {
    pub duration_s: Option<f64>,
    pub cuts_per_min: Option<f64>,
    pub motion_med: Option<f64>,
}

impl Features {
    pub(super) fn of(held: &Held) -> Self {
        let number = |name: &str| held.scores.get(name).and_then(Value::as_f64);
        Self {
            duration_s: number("duration_s")
                .or_else(|| Some(f64::from(held.duration_ms) / 1000.0).filter(|s| *s > 0.0)),
            cuts_per_min: number("cuts_per_min"),
            motion_med: number("motion_med"),
        }
    }

    pub fn kind(&self, label: VideoLabel) -> VideoLabel {
        match label {
            VideoLabel::Static => VideoLabel::Static,
            _ if self.duration_s.is_some_and(|s| s <= LOOP_MAX_S) => VideoLabel::Loop,
            _ => VideoLabel::Clip,
        }
    }

    fn line(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(value) = self.duration_s {
            parts.push(format!("duration {value:.0} s"));
        }
        if let Some(value) = self.cuts_per_min {
            parts.push(format!("scene cuts per minute {value:.1}"));
        }
        if let Some(value) = self.motion_med {
            parts.push(format!("median share of moving pixels {value:.3}"));
        }
        (!parts.is_empty()).then(|| format!("Measured on the file: {}.", parts.join(", ")))
    }
}

pub fn remote(frames: usize, features: &Features) -> String {
    let intro = format!(
        "These are {frames} frames from one video, in time order, spread evenly over its length."
    );
    let mut lines = vec![intro];
    lines.extend(features.line());
    lines.extend(["Classify the video:", STATIC, MOTION, RULES, JSON_REPLY].map(str::to_owned));
    lines.join("\n")
}

pub fn local(features: &Features) -> String {
    let intro = match features.duration_s {
        Some(seconds) => format!("{LOCAL_INTRO} The video is {seconds:.0} s long."),
        None => LOCAL_INTRO.to_owned(),
    };
    let mut lines = vec![intro];
    lines.extend(
        [
            "Classify the video as one of:",
            LOCAL_STATIC,
            LOCAL_CLIP,
            LOCAL_RULES,
            LOCAL_REPLY,
        ]
        .map(str::to_owned),
    );
    lines.join("\n")
}

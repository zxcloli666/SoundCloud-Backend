use serde_json::Value;

use super::queue::Held;

const DEFINITIONS: &str = "Classify the video:
static - no real filmed footage: one still picture or a few photos, album art, a logo, a drawn cover, a fan edit of photos, a screen recording, or a lyric video (song words on screen over a background, even a moving one). Effects on a still are not footage: zoom, pan, shake, flashes, glow, rain, particles, grain, color shifts, a waveform.
loop - one short piece of real motion, filmed or animated, that repeats. A file under 15 s with real motion is a loop. A long video is a loop only if the same few seconds of motion visibly come back.
clip - real filmed footage or real animation that goes on and changes: people or the camera move, shots change. A phone video of a person or a concert is a clip even if the framing barely changes.
Rules: lyrics written over the picture means static. The same background in every frame with only the text changing is static, not loop.";

const JSON_REPLY: &str =
    r#"Reply as JSON: {"reason": "<one short sentence>", "label": "static|loop|clip"}"#;

const WORD_REPLY: &str = "Answer with one word: clip, loop or static.";

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
    build(Some(frames), features, JSON_REPLY)
}

pub fn local(features: &Features) -> String {
    build(None, features, WORD_REPLY)
}

fn build(frames: Option<usize>, features: &Features, reply: &str) -> String {
    let intro = match frames {
        Some(n) => format!(
            "These are {n} frames from one video, in time order, spread evenly over its length."
        ),
        None => "These are frames from one video, in time order, spread evenly over its length."
            .to_owned(),
    };
    let mut lines = vec![intro];
    lines.extend(features.line());
    lines.push(DEFINITIONS.to_owned());
    lines.push(reply.to_owned());
    lines.join("\n")
}

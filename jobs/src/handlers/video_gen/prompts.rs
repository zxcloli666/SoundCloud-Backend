use serde_json::{Value, json};

use super::director::{Brief, Look, Plan, Review};

const MAX_PROMPT_CHARS: usize = 4_000;
const BASE_NEGATIVE: &str =
    "text, letters, watermark, logo, frame border, extra limbs, deformed face, blurry";

pub const SYSTEM: &str = "You direct a short square music video made from a track's cover art.
The pipeline is fixed: an image editor turns the cover into one keyframe per shot, an image-to-video model animates each keyframe for a few seconds, and the shots are cut together over the track's audio.
You never see the audio. You see the cover, the artist, the title, the duration and the lyrics when they exist.

Planning:
- Lock the character from the cover: gender, hair, clothes, and whether the face is visible. Every shot keeps that character. If the cover has no person, describe the main figure or object instead and set face_visible to false.
- Pick one mood and the bans: things that must never appear (text, logos, other people, styles that break the cover).
- Write exactly the number of shots asked for. Shots follow each other without gaps, each lasts from 1000 ms up to the stated maximum, and together they cover the strongest part of the song, usually the first chorus. Time the cuts to lyric lines when the lyrics have timestamps.
- keyframe is an edit instruction for the cover image: what changes in the scene, the framing and the light, while the character stays the same and the picture stays in the cover's style.
- motion is for the video model: one simple camera move and one simple action. No fast motion, no new characters, no text.

Reviewing:
- You get the frames to judge and can open the cover or earlier accepted keyframes with view_frame.
- Accept when the character matches the lock, nothing banned appears, and the picture is clean. For a shot, accept with trim_start_ms when the first part is weak.
- Reject with a concrete edit to the prompt when it fails. Answer only by calling accept or reject.";

pub fn plan_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["character", "mood", "bans", "shots"],
        "properties": {
            "character": {
                "type": "object",
                "additionalProperties": false,
                "required": ["gender", "hair", "clothes", "face_visible"],
                "properties": {
                    "gender": {"type": "string"},
                    "hair": {"type": "string"},
                    "clothes": {"type": "string"},
                    "face_visible": {"type": "boolean"}
                }
            },
            "mood": {"type": "string"},
            "bans": {"type": "array", "items": {"type": "string"}},
            "shots": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["start_ms", "end_ms", "line", "keyframe", "motion"],
                    "properties": {
                        "start_ms": {"type": "integer"},
                        "end_ms": {"type": "integer"},
                        "line": {"type": "string"},
                        "keyframe": {"type": "string"},
                        "motion": {"type": "string"}
                    }
                }
            }
        }
    })
}

pub fn review_tools() -> Value {
    json!([
        {
            "name": "view_frame",
            "description": "Open one image by name: cover, or keyframe{n} for an earlier accepted keyframe.",
            "input_schema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["name"],
                "properties": {"name": {"type": "string"}}
            }
        },
        {
            "name": "accept",
            "description": "Accept the frames under review. trim_start_ms cuts the start of a shot, 0 keeps it whole.",
            "input_schema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["trim_start_ms"],
                "properties": {"trim_start_ms": {"type": "integer"}}
            }
        },
        {
            "name": "reject",
            "description": "Reject the frames under review. reason says what is wrong, edit is the change to the prompt for the next try.",
            "input_schema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["reason", "edit"],
                "properties": {
                    "reason": {"type": "string"},
                    "edit": {"type": "string"}
                }
            }
        }
    ])
}

pub fn plan_request(brief: &Brief) -> String {
    let lyrics = brief
        .lyrics
        .as_deref()
        .unwrap_or("(no lyrics, treat it as instrumental)");
    format!(
        "Artist: {}\nTitle: {}\nDuration: {} ms\nShots: {}\nLongest shot: {} ms\n\nLyrics:\n{}",
        brief.artist, brief.title, brief.duration_ms, brief.shots, brief.shot_ms, lyrics
    )
}

pub fn review_request(review: &Review<'_>) -> String {
    let shot = &review.plan.shots[review.shot];
    let what = match review.look {
        Look::Keyframe => "the keyframe",
        Look::Shot => "stills from the start, middle and end of the animated shot",
    };
    let plan = serde_json::to_string(review.plan).unwrap_or_default();
    format!(
        "Plan: {plan}\n\nShot {} of {}, {}..{} ms, line: {}\nKeyframe prompt: {}\nMotion prompt: {}\n\nAttached is {what}. Earlier accepted keyframes: {}.",
        review.shot + 1,
        review.plan.shots.len(),
        shot.start_ms,
        shot.end_ms,
        shot.line,
        shot.keyframe,
        shot.motion,
        review.earlier.len()
    )
}

pub fn keyframe_prompt(plan: &Plan, shot: usize, edit: Option<&str>) -> String {
    let lock = character(plan);
    clipped(&format!(
        "{}\nKeep the character from the cover: {lock}. Keep the cover's style. Mood: {}.{}",
        plan.shots[shot].keyframe,
        plan.mood,
        edited(edit)
    ))
}

pub fn motion_prompt(plan: &Plan, shot: usize, edit: Option<&str>) -> String {
    let lock = character(plan);
    clipped(&format!(
        "{}\nThe same character: {lock}. Mood: {}.{}",
        plan.shots[shot].motion,
        plan.mood,
        edited(edit)
    ))
}

pub fn negative_prompt(plan: &Plan) -> String {
    let mut words = vec![BASE_NEGATIVE.to_owned()];
    words.extend(
        plan.bans
            .iter()
            .filter(|ban| !ban.trim().is_empty())
            .cloned(),
    );
    clipped(&words.join(", "))
}

fn character(plan: &Plan) -> String {
    let face = match plan.character.face_visible {
        true => "face visible",
        false => "face not shown",
    };
    format!(
        "{}, {}, {}, {face}",
        plan.character.gender, plan.character.hair, plan.character.clothes
    )
}

fn edited(edit: Option<&str>) -> String {
    edit.filter(|edit| !edit.trim().is_empty())
        .map(|edit| format!("\nChange: {edit}"))
        .unwrap_or_default()
}

fn clipped(text: &str) -> String {
    text.chars().take(MAX_PROMPT_CHARS).collect()
}

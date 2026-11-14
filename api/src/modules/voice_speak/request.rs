use backend_contracts::pipeline::MAX_SAY_TEXT_CHARS;
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const VOICES: [&str; 2] = ["loli", "moriarty"];
pub const BODY_LIMIT: usize = 4 * MAX_SAY_TEXT_CHARS as usize + 4 * 1024;

#[derive(Deserialize)]
pub struct SpeakBody {
    #[serde(default)]
    voice: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    keep: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub voice: &'static str,
    pub text: String,
    pub keep: bool,
}

impl SpeakBody {
    pub fn into_line(self) -> Result<Line, String> {
        let voice = VOICES
            .into_iter()
            .find(|known| *known == self.voice)
            .ok_or_else(|| format!("voice is not one of {}", VOICES.join(", ")))?;
        let text = self.text.trim();
        if text.is_empty() {
            return Err("text is empty".to_owned());
        }
        let most = MAX_SAY_TEXT_CHARS as usize;
        if text.chars().count() > most {
            return Err(format!("text is longer than {most} characters"));
        }
        Ok(Line {
            voice,
            text: text.to_owned(),
            keep: self.keep,
        })
    }
}

impl Line {
    pub fn phrase_key(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(self.voice.as_bytes());
        hash.update(b"\n");
        hash.update(self.text.as_bytes());
        format!("{}/{}.ogg", self.voice, hex::encode(hash.finalize()))
    }
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;

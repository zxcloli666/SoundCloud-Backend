use sc_chat_wire::proto::envelope::Body;
use sc_chat_wire::proto::{Embed, Envelope};

pub const LINE_CHARS_MAX: usize = 4_096;

pub fn render(envelope: &Envelope) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(forwarded) = &envelope.forwarded_from {
        parts.push(format!("[forwarded from {}]", forwarded.name));
    }
    if envelope.reply_to > 0 {
        parts.push(format!("[reply to #{}]", envelope.reply_to));
    }
    parts.push(body(envelope.body.as_ref()));
    for embed in &envelope.embeds {
        parts.push(embedded(embed));
    }
    if !envelope.attachments.is_empty() {
        parts.push(format!("[{} image(s)]", envelope.attachments.len()));
    }
    clip(&parts.join(" "))
}

pub fn system(body: &[u8]) -> String {
    clip(&format!("[system] {}", String::from_utf8_lossy(body)))
}

fn body(body: Option<&Body>) -> String {
    match body {
        Some(Body::Text(text)) => text.text.clone(),
        Some(Body::Edit(edit)) => format!(
            "[edit of #{}] {}",
            edit.target,
            edit.text
                .as_ref()
                .map(|text| text.text.as_str())
                .unwrap_or_default()
        ),
        Some(Body::Delete(delete)) => format!("[deleted #{}]", delete.target),
        Some(Body::Reaction(reaction)) => {
            format!("[reaction {} on #{}]", reaction.emoji, reaction.target)
        }
        Some(Body::Meta(meta)) => format!("[group info] {} {}", meta.title, meta.description),
        Some(Body::Pin(pin)) => format!("[pin #{}]", pin.target),
        Some(Body::Suggest(suggest)) => format!(
            "[suggested track] {}",
            suggest
                .track
                .as_ref()
                .map(|track| embedded(track))
                .unwrap_or_default()
        ),
        None => "[empty]".to_owned(),
    }
}

fn embedded(embed: &Embed) -> String {
    let mut shown = format!("[embed {} - {}", embed.title, embed.artist);
    if !embed.permalink.is_empty() {
        shown.push(' ');
        shown.push_str(&embed.permalink);
    }
    if let Some(room) = &embed.room {
        shown.push_str(&format!(" room {}", room.title));
    }
    shown.push(']');
    shown
}

fn clip(text: &str) -> String {
    text.chars().take(LINE_CHARS_MAX).collect()
}

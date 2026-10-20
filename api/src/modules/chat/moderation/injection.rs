const MARKERS: &[&str] = &[
    "ignore previous",
    "ignore all previous",
    "ignore the previous",
    "ignore the above",
    "ignore your instructions",
    "ignore all instructions",
    "ignore any instructions",
    "disregard previous",
    "disregard all",
    "disregard the above",
    "forget your instructions",
    "forget all instructions",
    "new instructions",
    "system prompt",
    "you are now",
    "developer mode",
    "as the moderator",
    "moderator instructions",
    "игнорируй",
    "проигнорируй",
    "забудь инструкции",
    "забудь все инструкции",
    "забудь предыдущие",
    "новые инструкции",
    "системный промпт",
    "ты теперь",
    "\"genuine\"",
    "genuine:",
    "genuine =",
    "recommended_action",
    "quote_seq",
    "<<<",
    ">>>",
];

pub fn suspected<'a>(texts: impl IntoIterator<Item = &'a str>) -> bool {
    texts.into_iter().any(|text| {
        let folded: String = text
            .to_lowercase()
            .chars()
            .map(|c| if c.is_whitespace() { ' ' } else { c })
            .collect();
        let squeezed = folded.split_whitespace().collect::<Vec<_>>().join(" ");
        MARKERS.iter().any(|marker| squeezed.contains(marker))
    })
}

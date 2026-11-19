use sc_mobile::{Caller, MobileClient, MobileError, MobileReply, streams};
use serde_json::Value;
use url::Url;

use crate::journal::{Journal, Sent, failure_note, json_of};

const FIRST_BYTES: usize = 2048;

struct Transcoding {
    url: Option<String>,
    preset: String,
    protocol: String,
    mime: String,
    quality: String,
    snipped: bool,
}

impl Transcoding {
    fn encrypted(&self) -> bool {
        self.protocol.contains("encrypted")
    }

    fn describe(&self) -> String {
        format!(
            "{} {} {} {}{}{}",
            self.preset,
            self.protocol,
            self.mime,
            self.quality,
            if self.encrypted() {
                " encrypted"
            } else {
                " clear"
            },
            if self.snipped { " snipped" } else { "" }
        )
    }

    fn preference(&self) -> u8 {
        match (self.snipped, self.encrypted(), self.protocol.as_str()) {
            (false, false, "progressive") => 0,
            (false, false, _) => 1,
            (false, true, _) => 2,
            (true, _, _) => 3,
        }
    }
}

pub async fn probe(
    client: &MobileClient,
    caller: &Caller,
    journal: &mut Journal,
    label: &str,
    track: &str,
) {
    let hls = streams::hls(track);
    let hls_outcome = client.call(caller, &hls).await;
    journal.reply(
        &format!("{label}.streams_hls"),
        Sent::of(&hls),
        &hls_outcome,
        stream_notes(&hls_outcome),
    );

    let https = streams::https(track);
    let https_outcome = client.call(caller, &https).await;
    journal.reply(
        &format!("{label}.streams_https"),
        Sent::of(&https),
        &https_outcome,
        stream_notes(&https_outcome),
    );

    let query = streams::transcodings(&[track]);
    let query_outcome = client.call(caller, &query).await;
    let transcodings = json_of(&query_outcome)
        .map(|body| transcodings_of(&body))
        .unwrap_or_default();
    let mut notes: Vec<String> = transcodings.iter().map(Transcoding::describe).collect();
    if let Some(access) = json_of(&query_outcome).as_ref().and_then(access_of) {
        notes.push(access);
    }
    notes.extend(failure_note(&query_outcome));
    journal.reply(
        &format!("{label}.transcodings"),
        Sent::of(&query),
        &query_outcome,
        notes,
    );

    let chosen = transcodings
        .iter()
        .filter(|transcoding| transcoding.url.is_some())
        .min_by_key(|transcoding| transcoding.preference());
    let stream_url = match chosen {
        Some(transcoding) => resolve(client, caller, journal, label, transcoding).await,
        None => https_outcome.as_ref().ok().and_then(streams::stream_url),
    };
    let Some(stream_url) = stream_url else {
        journal.skip(
            &format!("{label}.first_bytes"),
            "no stream URL was resolved",
        );
        return;
    };
    let outcome = client.first_bytes(&stream_url, FIRST_BYTES).await;
    let notes = match &outcome {
        Ok(reply) => vec![
            format!("{} bytes from {}", reply.body.len(), host_of(&stream_url)),
            sniff(&reply.body),
        ],
        Err(error) => vec![error.to_string()],
    };
    journal.reply(
        &format!("{label}.first_bytes"),
        Sent::get(&stream_url),
        &outcome,
        notes,
    );
}

async fn resolve(
    client: &MobileClient,
    caller: &Caller,
    journal: &mut Journal,
    label: &str,
    transcoding: &Transcoding,
) -> Option<Url> {
    let step = format!("{label}.media");
    let Some(url) = transcoding
        .url
        .as_deref()
        .and_then(|url| Url::parse(url).ok())
    else {
        journal.skip(&step, "the chosen transcoding URL does not parse");
        return None;
    };
    let request = streams::media(url.clone());
    let outcome = client.call(caller, &request).await;
    let mut notes = vec![format!("resolving {}", transcoding.describe())];
    if matches!(outcome, Err(MobileError::ForeignHost)) {
        notes.push(format!(
            "transcoding host {} is not a mobile API host",
            host_of(&url)
        ));
    }
    notes.extend(stream_notes(&outcome));
    let sent = Sent::get(&url);
    journal.reply(&step, sent, &outcome, notes);
    outcome.as_ref().ok().and_then(streams::stream_url)
}

fn stream_notes(outcome: &Result<MobileReply, MobileError>) -> Vec<String> {
    let Ok(reply) = outcome else {
        return failure_note(outcome).into_iter().collect();
    };
    let mut notes = Vec::new();
    if reply.is_captcha() {
        notes.push("captcha challenge, refused".to_owned());
        return notes;
    }
    if let Some(url) = streams::stream_url(reply) {
        notes.push(format!("stream on CDN host {}", host_of(&url)));
    } else if let Some(location) = &reply.location {
        notes.push(format!("redirect to {}", host_of(location)));
    }
    match serde_json::from_slice::<Value>(&reply.body) {
        Ok(Value::Object(fields)) => {
            let keys: Vec<&str> = fields.keys().map(String::as_str).collect();
            notes.push(format!("json keys {}", keys.join(",")));
        }
        Ok(_) => notes.push("json, not an object".to_owned()),
        Err(_) if !reply.body.is_empty() => notes.push(sniff(&reply.body)),
        Err(_) => {}
    }
    notes.extend(failure_note(outcome));
    notes
}

fn transcodings_of(body: &Value) -> Vec<Transcoding> {
    let Some(items) = body
        .pointer("/data/tracks/0/transcodings")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| Transcoding {
            url: text(item, "/url").map(str::to_owned),
            preset: text(item, "/preset").unwrap_or("?").to_owned(),
            protocol: text(item, "/format/protocol").unwrap_or("?").to_owned(),
            mime: text(item, "/format/mimeType").unwrap_or("?").to_owned(),
            quality: text(item, "/quality").unwrap_or("?").to_owned(),
            snipped: item
                .pointer("/snipped")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
        .collect()
}

fn access_of(body: &Value) -> Option<String> {
    let authorization = body.pointer("/data/tracks/0/authorization")?;
    let flag = |name: &str| {
        authorization
            .get(name)
            .map_or("?".to_owned(), Value::to_string)
    };
    Some(format!(
        "policy {} subMidTier {} subHighTier {} snipped {}",
        flag("policy"),
        flag("subMidTier"),
        flag("subHighTier"),
        flag("snipped")
    ))
}

fn text<'a>(item: &'a Value, pointer: &str) -> Option<&'a str> {
    item.pointer(pointer).and_then(Value::as_str)
}

fn host_of(url: &Url) -> String {
    url.host_str().unwrap_or("no host").to_owned()
}

fn sniff(body: &[u8]) -> String {
    let starts = |prefix: &[u8]| body.starts_with(prefix);
    if starts(b"#EXTM3U") {
        let playlist = String::from_utf8_lossy(body);
        match playlist
            .lines()
            .find(|line| line.starts_with("#EXT-X-KEY"))
            .and_then(|line| line.split("METHOD=").nth(1))
            .and_then(|method| method.split(',').next())
        {
            Some(method) => format!("HLS playlist, key method {method}"),
            None => "HLS playlist without a key".to_owned(),
        }
    } else if starts(b"ID3") || starts(&[0xff, 0xfb]) || starts(&[0xff, 0xf3]) {
        "MP3 audio".to_owned()
    } else if body.get(4..8) == Some(b"ftyp".as_slice()) {
        "MP4 audio".to_owned()
    } else if starts(b"OggS") {
        "Ogg audio".to_owned()
    } else if starts(b"{") || starts(b"[") {
        "JSON".to_owned()
    } else {
        "unrecognised bytes".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn transcodings_are_read_from_the_graphql_answer() {
        let body = json!({ "data": { "tracks": [{ "transcodings": [
            { "url": "https://api-mobile.soundcloud.com/media/a/b/stream/hls", "preset": "aac_160k",
              "format": { "protocol": "ctr-encrypted-hls", "mimeType": "audio/mp4" }, "quality": "hq", "snipped": false },
            { "url": "https://api-mobile.soundcloud.com/media/a/c/stream/progressive", "preset": "mp3_1_0",
              "format": { "protocol": "progressive", "mimeType": "audio/mpeg" }, "quality": "sq", "snipped": false },
        ] }] } });

        let found = transcodings_of(&body);

        assert_eq!(found.len(), 2);
        assert_eq!(
            found.first().map(Transcoding::describe).as_deref(),
            Some("aac_160k ctr-encrypted-hls audio/mp4 hq encrypted")
        );
        assert_eq!(
            found
                .iter()
                .min_by_key(|t| t.preference())
                .map(|t| t.preset.as_str()),
            Some("mp3_1_0")
        );
    }

    #[test]
    fn a_cdn_redirect_is_an_answer_that_names_its_host() {
        let redirect = MobileReply {
            status: 302,
            plan: None,
            location: Url::parse(
                "https://cf-hls-media.sndcdn.com/playlist/a.128.mp3/playlist.m3u8",
            )
            .ok(),
            body: Default::default(),
        };

        assert_eq!(
            stream_notes(&Ok(redirect)),
            vec!["stream on CDN host cf-hls-media.sndcdn.com".to_owned()]
        );
    }

    #[test]
    fn the_first_bytes_are_recognised() {
        assert_eq!(
            sniff(b"#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"x\"\n"),
            "HLS playlist, key method SAMPLE-AES"
        );
        assert_eq!(
            sniff(b"#EXTM3U\n#EXTINF:10,\n"),
            "HLS playlist without a key"
        );
        assert_eq!(sniff(b"ID3\x04"), "MP3 audio");
        assert_eq!(sniff(b"\0\0\0\x20ftypM4A "), "MP4 audio");
    }
}

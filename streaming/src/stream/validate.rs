pub fn looks_like_error_doc(bytes: &[u8]) -> bool {
    let trimmed = trim_ascii_start(bytes);
    matches!(trimmed.first(), Some(b'{') | Some(b'[') | Some(b'<'))
}

pub fn is_valid_audio(bytes: &[u8]) -> bool {
    !bytes.is_empty() && !looks_like_error_doc(bytes)
}

pub fn is_encrypted_mp4(bytes: &[u8]) -> bool {
    let mut at = 0usize;
    while let Some(header) = bytes.get(at..at + 8) {
        let size = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let end = match size {
            0 => bytes.len(),
            1..=7 => return false,
            _ => at.saturating_add(size).min(bytes.len()),
        };
        if &header[4..8] == b"moov" {
            return bytes[at..end]
                .windows(4)
                .any(|w| w == b"enca" || w == b"encv" || w == b"sinf");
        }
        at = end;
    }
    false
}

pub fn is_valid_m3u8(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let head_len = bytes.len().min(64);
    String::from_utf8_lossy(&bytes[..head_len]).contains("#EXTM3U")
}

fn trim_ascii_start(bytes: &[u8]) -> &[u8] {
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    &bytes[i..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_html_and_json_error_pages() {
        assert!(looks_like_error_doc(b"<!DOCTYPE html><html>blocked"));
        assert!(looks_like_error_doc(b"  \n {\"error\":\"forbidden\"}"));
        assert!(looks_like_error_doc(b"[]"));
        assert!(!looks_like_error_doc(b"ID3\x04\x00"));
        assert!(!looks_like_error_doc(&[0xFF, 0xFB, 0x90]));
        assert!(!looks_like_error_doc(b"\x00\x00\x00\x18ftypmp42"));
    }

    #[test]
    fn audio_validator() {
        assert!(!is_valid_audio(b""));
        assert!(!is_valid_audio(b"{\"url\":\"x\"}"));
        assert!(is_valid_audio(b"ID3\x04\x00\x00"));
    }

    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn a_cenc_init_is_encrypted_and_a_plain_one_is_not() {
        let ftyp = mp4_box(b"ftyp", b"iso6mp41");
        let mut encrypted = ftyp.clone();
        encrypted.extend(mp4_box(
            b"moov",
            &mp4_box(b"enca", &mp4_box(b"sinf", b"cenc")),
        ));
        encrypted.extend(mp4_box(b"moof", b"segment"));
        assert!(is_encrypted_mp4(&encrypted));

        let mut plain = ftyp;
        plain.extend(mp4_box(b"moov", &mp4_box(b"mp4a", b"esds")));
        plain.extend(mp4_box(b"mdat", b"enca in the audio is not a sample entry"));
        assert!(!is_encrypted_mp4(&plain));
    }

    #[test]
    fn mp3_and_adts_are_never_encrypted_mp4() {
        assert!(!is_encrypted_mp4(b"ID3\x04\x00\x00\x00\x00\x00\x00enca"));
        assert!(!is_encrypted_mp4(&[
            0xFF, 0xF1, 0x50, 0x80, 0x02, 0x1F, 0xFC, 0x21
        ]));
        assert!(!is_encrypted_mp4(b""));
    }

    #[test]
    fn m3u8_validator() {
        assert!(is_valid_m3u8(b"#EXTM3U\n#EXT-X-VERSION:3\nseg0.ts"));
        assert!(!is_valid_m3u8(b"<html>403</html>"));
        assert!(!is_valid_m3u8(b""));
    }
}

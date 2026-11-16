use super::*;

fn candidate(id: u32) -> MatchCandidate<'static> {
    MatchCandidate {
        id,
        artist: "Artist",
        title: "Song",
        duration_sec: Some(-5),
    }
}

fn reply(body: &str) -> anyhow::Result<MatchTrackData> {
    Ok(serde_json::from_str(body)?)
}

#[test]
fn the_request_stays_inside_the_contract_limits() -> anyhow::Result<()> {
    let candidates = (0..60).map(candidate).collect::<Vec<_>>();
    let target = MatchTarget {
        artist: "Artist",
        title: "Song",
    };

    let request = match_request(&target, &candidates);
    let encoded = serde_json::to_value(&request)?;

    assert_eq!(request.candidates.len(), MAX_MATCH_CANDIDATES as usize);
    assert_eq!(encoded["candidates"][0]["duration_sec"], Value::Null);
    assert_eq!(encoded["target"]["title"], "Song");
    Ok(())
}

#[test]
fn a_tie_is_no_match() -> anyhow::Result<()> {
    let tie = reply(r#"{"match_id":null,"confidence":0.62,"source":"deterministic"}"#)?;

    assert!(into_match(tie).is_none());
    Ok(())
}

#[test]
fn a_confident_match_is_kept_whatever_answered_it() -> anyhow::Result<()> {
    let deterministic = reply(r#"{"match_id":3,"confidence":0.99,"source":"deterministic"}"#)?;
    let llm = reply(r#"{"match_id":4,"confidence":0.1,"source":"llm"}"#)?;

    let picked = into_match(deterministic).ok_or_else(|| anyhow::anyhow!("no match"))?;
    let asked = into_match(llm).ok_or_else(|| anyhow::anyhow!("no match"))?;
    assert_eq!((picked.candidate_id, picked.confidence), (3, 0.95));
    assert_eq!((asked.candidate_id, asked.confidence), (4, 0.3));
    Ok(())
}

#[test]
fn a_cached_reply_without_its_source_is_not_read() {
    assert!(serde_json::from_str::<MatchTrackData>(r#"{"match_id":1,"confidence":0.9}"#).is_err());
}

#[test]
fn a_match_waits_no_longer_than_the_contract_window() {
    assert_eq!(match_timeout(20_000), Duration::from_secs(10));
    assert_eq!(match_timeout(4_000), Duration::from_secs(4));
    assert_eq!(match_timeout(1), MIN_TIMEOUT);
}

#[test]
fn the_cache_key_is_the_one_the_worker_replies_were_stored_under() {
    let target = MatchTarget {
        artist: "Artist",
        title: "Song",
    };
    let request = match_request(&target, &[candidate(0), candidate(1)]);

    assert_eq!(
        request_hash(&request),
        "IBJ6nBetlj2_kTypnNLIexMmgJonGHTV_KG-zWKPeaE"
    );
}

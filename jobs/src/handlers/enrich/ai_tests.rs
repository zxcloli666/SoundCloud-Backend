use super::*;
use crate::handlers::enrich::resolver::test_support::ctx;

const CONTRACT_EXAMPLE: &str = r#"{"primary_artist":"Kendrick Lamar","featured":["SZA"],"producers":[],"remixers":[],"album":null,"confidence":0.93,"source":"llm"}"#;

fn reply(source: &str) -> anyhow::Result<ResolveArtistData> {
    Ok(serde_json::from_str(&CONTRACT_EXAMPLE.replace(
        r#""source":"llm""#,
        &format!(r#""source":"{source}""#),
    ))?)
}

#[test]
fn a_language_model_answer_becomes_an_ai_attribution() -> anyhow::Result<()> {
    let track = ctx("All The Stars", Some("reupload channel"), None);

    let result = build_result(reply("llm")?, &track).ok_or_else(|| anyhow::anyhow!("no result"))?;

    assert_eq!(result.source, ResolveSource::Ai);
    assert_eq!(result.confidence, 0.75);
    assert_eq!(
        result.primary.first().map(|artist| artist.name.as_str()),
        Some("Kendrick Lamar")
    );
    assert_eq!(result.featured.len(), 1);
    Ok(())
}

#[test]
fn a_deterministic_answer_leaves_the_local_heuristic_in_charge() -> anyhow::Result<()> {
    let track = ctx("Kendrick Lamar - All The Stars", None, None);

    assert!(build_result(reply("deterministic")?, &track).is_none());
    Ok(())
}

#[test]
fn only_a_language_model_answer_is_remembered_for_a_month() {
    assert_eq!(answer_ttl(RpcSource::Llm), 30 * 24 * 60 * 60);
    assert_eq!(answer_ttl(RpcSource::Deterministic), 24 * 60 * 60);
}

#[test]
fn a_reply_without_its_source_is_not_read() {
    let legacy = r#"{"primary_artist":"Kendrick Lamar","featured":[],"producers":[],"remixers":[],"album":null,"confidence":0.93}"#;

    assert!(serde_json::from_str::<ResolveArtistData>(legacy).is_err());
}

#[test]
fn the_request_stays_inside_the_contract_limits() -> anyhow::Result<()> {
    let mut track = ctx("Song", Some("uploader"), Some("Artist"));
    track.description = Some("я".repeat(MAX_RESOLVE_DESCRIPTION_CHARS as usize + 50));
    track.duration_ms = Some(-1);

    let request = resolve_request(&track);
    let encoded = serde_json::to_value(&request)?;

    assert_eq!(
        request
            .description
            .map(|description| description.chars().count()),
        Some(MAX_RESOLVE_DESCRIPTION_CHARS as usize)
    );
    assert_eq!(request.duration_ms, None);
    assert_eq!(encoded["title"], "Song");
    assert_eq!(encoded["metadata_artist"], "Artist");
    Ok(())
}

#[test]
fn the_cache_key_changes_with_every_field_the_worker_reads() {
    let plain = ctx("Song", Some("uploader"), None);
    let with_artist = ctx("Song", Some("uploader"), Some("Artist"));
    let mut with_description = ctx("Song", Some("uploader"), None);
    with_description.description = Some("prod. by someone".to_owned());

    assert_ne!(context_hash(&plain), context_hash(&with_artist));
    assert_ne!(context_hash(&plain), context_hash(&with_description));
    assert_eq!(
        context_hash(&plain),
        context_hash(&ctx("Song", Some("uploader"), None))
    );
}

#[test]
fn a_silent_or_failing_ai_lane_is_a_source_failure_not_an_empty_answer() -> anyhow::Result<()> {
    let silent = answered(Ok(None));
    let failing = answered(Err(anyhow::anyhow!("NATS request timed out")));
    let answer = answered(Ok(Some(reply("llm")?)))?;

    assert!(matches!(
        silent,
        Err(EnrichError::Source(SourceError::Unreachable(_)))
    ));
    assert!(matches!(
        failing,
        Err(EnrichError::Source(SourceError::Unreachable(_)))
    ));
    assert_eq!(answer.primary_artist.as_deref(), Some("Kendrick Lamar"));
    Ok(())
}

#[test]
fn a_resolve_waits_no_longer_than_the_contract_window() {
    assert_eq!(resolve_timeout(60_000), Duration::from_secs(20));
    assert_eq!(resolve_timeout(5_000), Duration::from_secs(5));
    assert_eq!(resolve_timeout(1), MIN_TIMEOUT);
}

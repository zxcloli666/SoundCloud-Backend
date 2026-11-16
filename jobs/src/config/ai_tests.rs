use std::io::Write;

use super::*;

const LADDERS: &str = r#"
[providers.alpha]
kind = "openai_compatible"
endpoint = "https://example.invalid/v1/chat/completions"
model = "example-model"
api_key = "env:EXAMPLE_ALPHA_KEY"
max_output_tokens = 400
total_timeout_ms = 6000

[providers.onsite]
kind = "worker"
subject = "ai.rpc.complete"
max_output_tokens = 400
total_timeout_ms = 9000
hedge = false

[ladders.refine]
steps = ["alpha", "onsite"]
hedge_after_ms = 3000
deadline_ms = 10000
breaker = { failures = 4, window_s = 60, open_s = 120 }
"#;

fn written(text: &str) -> anyhow::Result<tempfile::NamedTempFile> {
    let mut file = tempfile::NamedTempFile::new()?;
    file.write_all(text.as_bytes())?;
    Ok(file)
}

fn refused(result: Result<impl std::fmt::Debug, ConfigError>) -> String {
    match result {
        Ok(value) => format!("accepted {value:?}"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn a_named_ladder_turns_the_refinement_on() -> anyhow::Result<()> {
    let file = written(LADDERS)?;

    let settings = load(file.path())?;
    let on = refine(true, Some(settings), "refine".to_owned())?
        .ok_or_else(|| anyhow::anyhow!("the refinement stayed off"))?;

    assert_eq!(on.ladder, "refine");
    assert_eq!(on.settings.providers.len(), 2);
    Ok(())
}

#[test]
fn the_refinement_is_off_without_a_file_or_when_disabled() -> anyhow::Result<()> {
    let file = written(LADDERS)?;
    let settings = load(file.path())?;

    let disabled = refine(false, Some(settings), "refine".to_owned())?;
    let unset = refine(true, None, "refine".to_owned())?;

    assert!(disabled.is_none());
    assert!(unset.is_none());
    Ok(())
}

#[test]
fn a_ladder_the_file_does_not_name_stops_the_start() -> anyhow::Result<()> {
    let file = written(LADDERS)?;
    let settings = load(file.path())?;

    let reason = refused(refine(true, Some(settings), "missing".to_owned()));

    assert!(reason.contains("ENRICH_AI_LADDER"), "{reason}");
    assert!(reason.contains("no ladder missing"), "{reason}");
    Ok(())
}

#[test]
fn an_unreadable_or_invalid_file_stops_the_start_with_its_reason() -> anyhow::Result<()> {
    let unknown_section = written(&format!("{LADDERS}\n[voice_chat]\nladder = \"refine\"\n"))?;
    let literal_key = written(&LADDERS.replace("env:EXAMPLE_ALPHA_KEY", "plain-key"))?;
    let broken = written("[ladders.refine\n")?;

    for (path, expected) in [
        (Path::new("/nonexistent/llm.toml"), "unreadable"),
        (unknown_section.path(), "voice_chat"),
        (literal_key.path(), "env:NAME"),
        (broken.path(), "not valid"),
    ] {
        let reason = refused(load(path));

        assert!(reason.starts_with("invalid LLM_CONFIG"), "{reason}");
        assert!(reason.contains(expected), "{reason}");
    }
    Ok(())
}

#[test]
fn the_shipped_example_names_the_default_ladder() -> anyhow::Result<()> {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("llm.toml.example");

    let settings = load(&example)?;

    assert!(refine(true, Some(settings), DEFAULT_LADDER.to_owned())?.is_some());
    Ok(())
}

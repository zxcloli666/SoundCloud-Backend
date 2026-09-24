use catalog_normalize::name_in;

use super::signals::{LocalSignals, name_matches_uploader};
use super::{ArtistCandidate, CreditEvidence, ResolveResult, ResolveSource, TrackContext};

pub(super) fn merge_with(
    heuristic: ResolveResult,
    ext_res: ResolveResult,
    ctx: &TrackContext,
    signals: &LocalSignals,
) -> ResolveResult {
    let mut out = ext_res;
    if out.primary.is_empty() {
        out.primary = heuristic.primary.clone();
    } else {
        attach_uploader_id(&mut out.primary, ctx);
    }
    if out.producers.is_empty() {
        out.producers = heuristic.producers;
    }
    if out.remixers.is_empty() {
        out.remixers = heuristic.remixers;
    }
    if out.featured.is_empty() {
        out.featured = heuristic.featured;
    }

    if let Some(markup) = signals.markup() {
        extend_missing_coprimary(&mut out, markup, ctx);
    }
    extend_missing_coprimary(&mut out, &signals.meta_names, ctx);
    out
}

pub fn enrich_with_local_signals(
    fast: ResolveResult,
    ctx: &TrackContext,
    signals: &LocalSignals,
) -> ResolveResult {
    merge_with(signals.heuristic(ctx), fast, ctx, signals)
}

fn attach_uploader_id(primary: &mut [ArtistCandidate], ctx: &TrackContext) {
    let Some(sc_id) = ctx.uploader_sc_user_id.as_deref() else {
        return;
    };
    for cand in primary.iter_mut() {
        if cand.sc_user_id.is_none()
            && name_matches_uploader(&cand.name, ctx.uploader_username.as_deref())
        {
            cand.sc_user_id = Some(sc_id.to_string());
        }
    }
}

fn extend_missing_coprimary(out: &mut ResolveResult, names: &[String], ctx: &TrackContext) {
    if out.primary.is_empty() || names.len() < 2 {
        return;
    }
    let primary_names: Vec<String> = out.primary.iter().map(|c| c.name.clone()).collect();
    let agrees = names
        .iter()
        .any(|m| name_in(m, primary_names.iter().map(|s| s.as_str())));
    if !agrees {
        return;
    }
    let known: Vec<String> = out
        .primary
        .iter()
        .chain(out.featured.iter())
        .chain(out.producers.iter())
        .chain(out.remixers.iter())
        .map(|c| c.name.clone())
        .collect();
    for m in names {
        if !name_in(m, known.iter().map(|s| s.as_str())) {
            let sc_user_id = if name_matches_uploader(m, ctx.uploader_username.as_deref()) {
                ctx.uploader_sc_user_id.clone()
            } else {
                None
            };
            out.primary.push(
                ArtistCandidate {
                    name: m.clone(),
                    mb_id: None,
                    genius_id: None,
                    sc_user_id,
                    ..Default::default()
                }
                .attributed(
                    ResolveSource::Meta,
                    0.5,
                    CreditEvidence::MetadataField,
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{ctx, names, run_heuristic, signals_no_dict};
    use super::*;
    use crate::handlers::enrich::resolver::{ResolveSource, TrackContext};

    fn merge_for(c: &TrackContext, ext: ResolveResult) -> ResolveResult {
        let signals = signals_no_dict(c);
        merge_with(run_heuristic(c), ext, c, &signals)
    }

    #[test]
    fn merge_appends_missing_meta_coprimary() {
        let c = ctx("паралич", Some("Psychosis"), Some("Psychosis, killaheelz"));
        let ext = ResolveResult {
            source: ResolveSource::Genius,
            confidence: 0.8,
            primary: vec![ArtistCandidate {
                name: "Psychosis".into(),
                mb_id: None,
                genius_id: Some("123".into()),
                sc_user_id: None,
                ..Default::default()
            }],
            ..Default::default()
        };
        let merged = merge_for(&c, ext);
        assert_eq!(names(&merged), vec!["Psychosis", "killaheelz"]);
        assert_eq!(merged.source, ResolveSource::Genius);
    }

    #[test]
    fn merge_skips_meta_when_disjoint_from_external() {
        let c = ctx("song", Some("up"), Some("Akio Ohmori, Ritsuo Kamimura"));
        let ext = ResolveResult {
            source: ResolveSource::Genius,
            confidence: 0.7,
            primary: vec![ArtistCandidate {
                name: "Cyalm".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let merged = merge_for(&c, ext);
        assert_eq!(names(&merged), vec!["Cyalm"]);
    }

    #[test]
    fn markup_coartists_added_to_external_result() {
        let c = ctx(
            "Psychosis, LEYNCLOUD, inxwertg - blade mail",
            Some("0n3PunchMan"),
            None,
        );
        let ext = ResolveResult {
            source: ResolveSource::Genius,
            confidence: 0.8,
            primary: vec![ArtistCandidate {
                name: "Psychosis".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let merged = merge_for(&c, ext);
        assert_eq!(names(&merged), vec!["Psychosis", "LEYNCLOUD", "inxwertg"]);
    }

    #[test]
    fn fast_path_enriched_with_markup_and_meta() {
        let c = ctx("мокери, psychosis - no.happiness", Some("МОКЕРИ"), None);
        let fast = ResolveResult {
            source: ResolveSource::ScVerified,
            confidence: 1.0,
            primary: vec![ArtistCandidate {
                name: "МОКЕРИ".into(),
                sc_user_id: Some("42".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let signals = signals_no_dict(&c);
        let out = enrich_with_local_signals(fast, &c, &signals);
        assert_eq!(names(&out), vec!["МОКЕРИ", "psychosis"]);
        assert_eq!(out.source, ResolveSource::ScVerified);

        let c2 = ctx("без шансов", Some("dekma"), Some("takizava & dekma"));
        let fast2 = ResolveResult {
            source: ResolveSource::ScVerified,
            confidence: 1.0,
            primary: vec![ArtistCandidate {
                name: "dekma".into(),
                sc_user_id: Some("42".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let signals2 = signals_no_dict(&c2);
        let out2 = enrich_with_local_signals(fast2, &c2, &signals2);
        assert_eq!(names(&out2), vec!["dekma", "takizava"]);
    }

    #[test]
    fn fast_path_gets_unreversed_markup() {
        let c = ctx(
            "505 - arctic monkeys",
            Some("arcticmonkeys"),
            Some("Arctic Monkeys"),
        );
        let fast = ResolveResult {
            source: ResolveSource::ScVerified,
            confidence: 1.0,
            primary: vec![ArtistCandidate {
                name: "Arctic Monkeys".into(),
                sc_user_id: Some("42".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let signals = signals_no_dict(&c);
        let out = enrich_with_local_signals(fast, &c, &signals);
        assert_eq!(names(&out), vec!["Arctic Monkeys"]);
    }

    #[test]
    fn merge_does_not_duplicate_featured_from_meta() {
        let c = ctx(
            "GLAM GO! - ГЛЯНЬ ЕЙ НА ЛИЦО (feat. Gone.Fludd)",
            Some("glamgo"),
            Some("Glam Go, Gone.Fludd"),
        );
        let ext = ResolveResult {
            source: ResolveSource::Genius,
            confidence: 0.8,
            primary: vec![ArtistCandidate {
                name: "GLAM GO GANG!".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let merged = merge_for(&c, ext);
        assert_eq!(names(&merged), vec!["GLAM GO GANG!"]);
        assert!(
            merged
                .featured
                .iter()
                .any(|f| f.name.to_lowercase().contains("fludd"))
        );
    }

    #[test]
    fn uploader_id_attached_to_any_position() {
        let c = ctx("A, uploader - song", Some("uploader"), None);
        let ext = ResolveResult {
            source: ResolveSource::Genius,
            confidence: 0.8,
            primary: vec![
                ArtistCandidate {
                    name: "A".into(),
                    ..Default::default()
                },
                ArtistCandidate {
                    name: "uploader".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let merged = merge_for(&c, ext);
        assert_eq!(merged.primary[1].sc_user_id.as_deref(), Some("42"));
    }
}

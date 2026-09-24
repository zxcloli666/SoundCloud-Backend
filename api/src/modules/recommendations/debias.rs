use super::service::RecommendResult;

pub fn ips_debias(items: &mut [RecommendResult]) {
    for it in items.iter_mut() {
        let plays = it.playback_count.unwrap_or(0).max(0) as f64;
        let denom = (1.0 + plays.ln_1p()).sqrt() as f32;
        if denom > 1.0
            && let Some(s) = it.score.as_mut()
        {
            *s /= denom;
        }
    }
    items.sort_by(|a, b| {
        b.score
            .unwrap_or(0.0)
            .partial_cmp(&a.score.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(id: u64, score: Option<f32>, plays: Option<i64>) -> RecommendResult {
        RecommendResult {
            id: json!(id),
            score,
            payload: None,
            artist: None,
            genre: None,
            playback_count: plays,
            features: None,
        }
    }

    fn order(items: &[RecommendResult]) -> Vec<u64> {
        items
            .iter()
            .map(|item| item.id.as_u64().expect("test ids are numbers"))
            .collect()
    }

    #[test]
    fn a_hit_and_an_obscure_track_with_the_same_score_do_not_stay_tied() {
        let mut items = vec![
            item(1, Some(0.9), Some(5_000_000)),
            item(2, Some(0.9), Some(3)),
        ];

        ips_debias(&mut items);

        assert_eq!(
            order(&items),
            vec![2, 1],
            "popularity must be divided out, otherwise the wave is a chart"
        );
    }

    #[test]
    fn a_track_nobody_played_keeps_its_score_untouched() {
        let mut items = vec![item(1, Some(0.5), Some(0)), item(2, Some(0.4), None)];

        ips_debias(&mut items);

        assert_eq!(items[0].score, Some(0.5));
        assert_eq!(items[1].score, Some(0.4));
    }

    #[test]
    fn a_negative_play_count_cannot_lift_a_score() {
        let mut items = vec![item(1, Some(0.5), Some(-1_000))];

        ips_debias(&mut items);

        assert_eq!(
            items[0].score,
            Some(0.5),
            "a broken counter must not become a ranking bonus"
        );
    }

    #[test]
    fn an_item_without_a_score_sinks_below_every_scored_one() {
        let mut items = vec![item(1, None, Some(10)), item(2, Some(0.01), Some(10))];

        ips_debias(&mut items);

        assert_eq!(order(&items), vec![2, 1]);
    }

    #[test]
    fn popularity_is_damped_not_erased() {
        let mut items = vec![
            item(1, Some(1.0), Some(1_000_000)),
            item(2, Some(0.2), Some(1)),
        ];

        ips_debias(&mut items);

        assert_eq!(
            order(&items),
            vec![1, 2],
            "a much better score must still win, the penalty is a divisor and not a veto"
        );
    }
}

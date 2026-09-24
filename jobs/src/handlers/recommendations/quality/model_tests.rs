use super::*;

const SKLEARN_WEIGHTS: [f64; FEATURE_COUNT] = [
    2.47655, -1.13792, 0.1086, 0.33357, 0.48905, 0.05406, 0.05811, -0.53591, 0.87393, 0.0,
];
const SKLEARN_INTERCEPT: f64 = 0.76335;
const SKLEARN_ACCURACY: f64 = 0.925;
const SKLEARN_FIRST_PROBABILITIES: [f32; 3] = [0.99694, 0.90256, 0.00476];

fn synthetic(count: u64) -> Vec<LabeledTrack> {
    (0..count)
        .map(|index| {
            let mut values = [1.0_f32; FEATURE_COUNT];
            for (column, value) in values.iter_mut().take(FEATURE_COUNT - 1).enumerate() {
                let key = index * 10 + column as u64 + 1;
                let hash = (key * 2_654_435_761) % (1 << 32);
                *value = (((hash >> 8) % 1000) as f64 / 1000.0) as f32;
            }
            let noise = ((index * 7919) % 100) as f64 / 100.0 - 0.5;
            let [first, second, _, _, fifth, ..] = values.map(f64::from);
            let signal = 2.0 * first - second + 0.5 * fifth + noise;
            LabeledTrack {
                features: QualityFeatures::from_values(values),
                positive: signal > 0.6,
            }
        })
        .collect()
}

fn trained(examples: &[LabeledTrack]) -> anyhow::Result<TrainedModel> {
    train(examples).map_err(|refusal| anyhow::anyhow!("training refused: {}", refusal.as_str()))
}

#[test]
fn the_fit_matches_scikit_learn_with_balanced_classes() -> anyhow::Result<()> {
    let examples = synthetic(240);
    let result = trained(&examples)?;

    assert!(
        result.converged,
        "stopped after {} iterations",
        result.iterations
    );
    assert_eq!(result.positives, 158);
    for (weight, expected) in result.model.weights.iter().zip(SKLEARN_WEIGHTS) {
        assert!(
            (f64::from(*weight) - expected).abs() < 1e-3,
            "weight {weight} differs from scikit-learn {expected}"
        );
    }
    assert!((f64::from(result.model.intercept) - SKLEARN_INTERCEPT).abs() < 1e-3);
    assert!((result.accuracy - SKLEARN_ACCURACY).abs() < 1e-9);
    for (example, expected) in examples.iter().zip(SKLEARN_FIRST_PROBABILITIES) {
        assert!((result.model.score(&example.features) - expected).abs() < 1e-3);
    }
    Ok(())
}

#[test]
fn the_fit_is_the_minimum_of_the_regularized_loss() -> anyhow::Result<()> {
    let examples = synthetic(240);
    let positives = examples.iter().filter(|example| example.positive).count();
    let scaler = Scaler::fit(&examples);
    let dataset = Dataset::new(&examples, &scaler, positives, examples.len() - positives);
    let fit = dataset.minimize();

    assert!(dataset.gradient(&fit.params).largest_magnitude() < 1e-6);
    let best = dataset.objective(&fit.params);
    for shift in [-0.05, 0.05] {
        let nudged = Params {
            weights: fit.params.weights.map(|weight| weight + shift),
            intercept: fit.params.intercept + shift,
        };
        assert!(dataset.objective(&nudged) > best);
    }
    Ok(())
}

#[test]
fn a_constant_feature_keeps_a_unit_scale_and_no_weight() -> anyhow::Result<()> {
    let result = trained(&synthetic(240))?;

    assert_eq!(result.model.means.last(), Some(&1.0));
    assert_eq!(result.model.scales.last(), Some(&1.0));
    assert_eq!(result.model.weights.last(), Some(&0.0));
    Ok(())
}

#[test]
fn a_small_dataset_is_not_trained() {
    let examples = synthetic(u64::try_from(MIN_EXAMPLES - 1).unwrap_or_default());

    assert_eq!(
        train(&examples).err(),
        Some(TrainingRefusal::TooFewExamples)
    );
}

#[test]
fn labels_from_one_side_are_not_trained() {
    let examples = synthetic(240)
        .into_iter()
        .map(|example| LabeledTrack {
            features: example.features,
            positive: true,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        train(&examples).err(),
        Some(TrainingRefusal::OneSidedLabels)
    );
}

#[test]
fn a_score_is_a_probability() -> anyhow::Result<()> {
    let neutral = QualityModel {
        means: [0.0; FEATURE_COUNT],
        scales: [1.0; FEATURE_COUNT],
        weights: [0.0; FEATURE_COUNT],
        intercept: 0.0,
    };
    let features = QualityFeatures::from_values([0.3; FEATURE_COUNT]);
    let extreme = QualityModel {
        weights: [1.0e6; FEATURE_COUNT],
        ..neutral.clone()
    };

    assert_eq!(neutral.score(&features), 0.5);
    assert_eq!(extreme.score(&features), 1.0);
    assert!((0.0..=1.0).contains(&trained(&synthetic(240))?.model.score(&features)));
    Ok(())
}

#[test]
fn a_model_with_a_broken_scale_is_not_usable() {
    let usable = QualityModel {
        means: [0.0; FEATURE_COUNT],
        scales: [1.0; FEATURE_COUNT],
        weights: [0.5; FEATURE_COUNT],
        intercept: 0.1,
    };
    let zero_scale = QualityModel {
        scales: [0.0; FEATURE_COUNT],
        ..usable.clone()
    };
    let infinite_weight = QualityModel {
        weights: [f32::INFINITY; FEATURE_COUNT],
        ..usable.clone()
    };

    assert!(usable.is_usable());
    assert!(!zero_scale.is_usable());
    assert!(!infinite_weight.is_usable());
}

use super::features::{FEATURE_COUNT, QualityFeatures};

pub const MIN_EXAMPLES: usize = 100;
const MIN_EXAMPLES_PER_CLASS: usize = 10;
const INVERSE_REGULARIZATION: f64 = 1.0;
const MAX_ITERATIONS: usize = 50_000;
const GRADIENT_TOLERANCE: f64 = 1e-7;
const SMALLEST_SCALE: f64 = 1e-12;

type Row = [f64; FEATURE_COUNT];

#[derive(Clone, Debug, PartialEq)]
pub struct QualityModel {
    pub means: [f32; FEATURE_COUNT],
    pub scales: [f32; FEATURE_COUNT],
    pub weights: [f32; FEATURE_COUNT],
    pub intercept: f32,
}

impl QualityModel {
    pub fn score(&self, features: &QualityFeatures) -> f32 {
        let logit = features
            .as_array()
            .iter()
            .zip(&self.means)
            .zip(&self.scales)
            .zip(&self.weights)
            .map(|(((value, mean), scale), weight)| {
                f64::from(*weight) * (f64::from(*value) - f64::from(*mean)) / f64::from(*scale)
            })
            .sum::<f64>()
            + f64::from(self.intercept);
        sigmoid(logit) as f32
    }

    pub fn is_usable(&self) -> bool {
        self.intercept.is_finite()
            && self.means.iter().all(|mean| mean.is_finite())
            && self.weights.iter().all(|weight| weight.is_finite())
            && self
                .scales
                .iter()
                .all(|scale| scale.is_finite() && *scale > 0.0)
    }
}

pub struct LabeledTrack {
    pub features: QualityFeatures,
    pub positive: bool,
}

#[derive(Debug)]
pub struct TrainedModel {
    pub model: QualityModel,
    pub examples: usize,
    pub positives: usize,
    pub accuracy: f64,
    pub iterations: usize,
    pub converged: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrainingRefusal {
    TooFewExamples,
    OneSidedLabels,
    UnusableModel,
}

impl TrainingRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TooFewExamples => "too_few_examples",
            Self::OneSidedLabels => "one_sided_labels",
            Self::UnusableModel => "unusable_model",
        }
    }
}

pub fn train(examples: &[LabeledTrack]) -> Result<TrainedModel, TrainingRefusal> {
    let positives = examples.iter().filter(|example| example.positive).count();
    let negatives = examples.len() - positives;
    if examples.len() < MIN_EXAMPLES {
        return Err(TrainingRefusal::TooFewExamples);
    }
    if positives.min(negatives) < MIN_EXAMPLES_PER_CLASS {
        return Err(TrainingRefusal::OneSidedLabels);
    }

    let scaler = Scaler::fit(examples);
    let fit = Dataset::new(examples, &scaler, positives, negatives).minimize();
    let model = QualityModel {
        means: narrow(&scaler.means),
        scales: narrow(&scaler.scales),
        weights: narrow(&fit.params.weights),
        intercept: fit.params.intercept as f32,
    };
    if !model.is_usable() {
        return Err(TrainingRefusal::UnusableModel);
    }

    let correct = examples
        .iter()
        .filter(|example| (model.score(&example.features) >= 0.5) == example.positive)
        .count();
    Ok(TrainedModel {
        model,
        examples: examples.len(),
        positives,
        accuracy: correct as f64 / examples.len() as f64,
        iterations: fit.iterations,
        converged: fit.converged,
    })
}

struct Scaler {
    means: Row,
    scales: Row,
}

impl Scaler {
    fn fit(examples: &[LabeledTrack]) -> Self {
        let count = examples.len() as f64;
        let mut sums = [0.0; FEATURE_COUNT];
        for example in examples {
            for (sum, value) in sums.iter_mut().zip(widen(&example.features)) {
                *sum += value;
            }
        }
        let means = sums.map(|sum| sum / count);
        let mut squared_deviations = [0.0; FEATURE_COUNT];
        for example in examples {
            for ((squared, value), mean) in squared_deviations
                .iter_mut()
                .zip(widen(&example.features))
                .zip(&means)
            {
                *squared += (value - mean).powi(2);
            }
        }
        let scales = squared_deviations.map(|squared| {
            let deviation = (squared / count).sqrt();
            if deviation > SMALLEST_SCALE {
                deviation
            } else {
                1.0
            }
        });
        Self { means, scales }
    }

    fn transform(&self, features: &QualityFeatures) -> Row {
        let mut row = widen(features);
        for ((value, mean), scale) in row.iter_mut().zip(&self.means).zip(&self.scales) {
            *value = (*value - mean) / scale;
        }
        row
    }
}

#[derive(Clone, Copy, Debug)]
struct Params {
    weights: Row,
    intercept: f64,
}

impl Params {
    const ZERO: Self = Self {
        weights: [0.0; FEATURE_COUNT],
        intercept: 0.0,
    };

    fn logit(&self, row: &Row) -> f64 {
        dot(&self.weights, row) + self.intercept
    }

    fn plus(&self, other: &Self, factor: f64) -> Self {
        let mut weights = self.weights;
        for (weight, delta) in weights.iter_mut().zip(&other.weights) {
            *weight += factor * delta;
        }
        Self {
            weights,
            intercept: self.intercept + factor * other.intercept,
        }
    }

    fn largest_magnitude(&self) -> f64 {
        self.weights
            .iter()
            .fold(self.intercept.abs(), |largest, weight| {
                largest.max(weight.abs())
            })
    }
}

struct Sample {
    row: Row,
    target: f64,
    weight: f64,
}

struct Dataset {
    samples: Vec<Sample>,
    ridge: f64,
}

struct Fit {
    params: Params,
    iterations: usize,
    converged: bool,
}

impl Dataset {
    fn new(examples: &[LabeledTrack], scaler: &Scaler, positives: usize, negatives: usize) -> Self {
        let count = examples.len() as f64;
        let positive_weight = count / (2.0 * positives as f64);
        let negative_weight = count / (2.0 * negatives as f64);
        let samples = examples
            .iter()
            .map(|example| Sample {
                row: scaler.transform(&example.features),
                target: if example.positive { 1.0 } else { 0.0 },
                weight: if example.positive {
                    positive_weight
                } else {
                    negative_weight
                },
            })
            .collect();
        Self {
            samples,
            ridge: 1.0 / (INVERSE_REGULARIZATION * count),
        }
    }

    fn minimize(&self) -> Fit {
        let step = 1.0 / self.smoothness();
        let mut current = Params::ZERO;
        let mut previous = Params::ZERO;
        for iteration in 1..=MAX_ITERATIONS {
            let momentum = (iteration as f64 - 1.0) / (iteration as f64 + 2.0);
            let lookahead = current.plus(&current.plus(&previous, -1.0), momentum);
            let gradient = self.gradient(&lookahead);
            previous = current;
            current = lookahead.plus(&gradient, -step);
            if gradient.largest_magnitude() < GRADIENT_TOLERANCE {
                return Fit {
                    params: current,
                    iterations: iteration,
                    converged: true,
                };
            }
        }
        Fit {
            params: current,
            iterations: MAX_ITERATIONS,
            converged: false,
        }
    }

    fn gradient(&self, params: &Params) -> Params {
        let count = self.samples.len() as f64;
        let penalty = Params {
            weights: params.weights,
            intercept: 0.0,
        };
        let mut gradient = Params::ZERO.plus(&penalty, self.ridge);
        for sample in &self.samples {
            let residual = sample.weight * (sigmoid(params.logit(&sample.row)) - sample.target);
            let direction = Params {
                weights: sample.row,
                intercept: 1.0,
            };
            gradient = gradient.plus(&direction, residual / count);
        }
        gradient
    }

    fn smoothness(&self) -> f64 {
        let count = self.samples.len() as f64;
        self.samples
            .iter()
            .map(|sample| sample.weight * (dot(&sample.row, &sample.row) + 1.0))
            .sum::<f64>()
            / (4.0 * count)
            + self.ridge
    }

    #[cfg(test)]
    fn objective(&self, params: &Params) -> f64 {
        let count = self.samples.len() as f64;
        let loss = self
            .samples
            .iter()
            .map(|sample| {
                let probability = sigmoid(params.logit(&sample.row)).clamp(1e-15, 1.0 - 1e-15);
                -sample.weight
                    * (sample.target * probability.ln()
                        + (1.0 - sample.target) * (1.0 - probability).ln())
            })
            .sum::<f64>()
            / count;
        loss + self.ridge / 2.0 * dot(&params.weights, &params.weights)
    }
}

fn sigmoid(logit: f64) -> f64 {
    if logit >= 0.0 {
        1.0 / (1.0 + (-logit).exp())
    } else {
        let exponent = logit.exp();
        exponent / (1.0 + exponent)
    }
}

fn dot(left: &Row, right: &Row) -> f64 {
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

fn widen(features: &QualityFeatures) -> Row {
    features.as_array().map(f64::from)
}

fn narrow(values: &Row) -> [f32; FEATURE_COUNT] {
    values.map(|value| value as f32)
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;

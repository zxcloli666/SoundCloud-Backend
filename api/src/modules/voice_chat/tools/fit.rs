use serde_json::{Map, Number, Value};

use super::{Action, Arg, Kind, Range, Tools};

impl Tools {
    pub fn fit(&self, raw: Value) -> Option<Action> {
        let Value::Object(mut raw) = raw else {
            return None;
        };
        let tool = self.named(raw.get("name")?.as_str()?)?;
        let given = given(raw.remove("args"))?;
        let mut args = Map::new();
        for arg in &tool.args {
            match given.get(&arg.name).filter(|value| !arg.left_out(value)) {
                Some(value) => {
                    args.insert(arg.name.clone(), arg.kind.fit(value)?);
                }
                None if arg.required => return None,
                None => {}
            }
        }
        Some(Action {
            name: tool.name.clone(),
            args,
        })
    }
}

impl Arg {
    fn left_out(&self, value: &Value) -> bool {
        match value {
            Value::Null => true,
            Value::String(text) => !self.required && text.trim().is_empty(),
            _ => false,
        }
    }
}

impl Kind {
    fn fit(&self, value: &Value) -> Option<Value> {
        match self {
            Self::Text => match value {
                Value::String(text) => Some(Value::String(text.clone())),
                Value::Number(number) => Some(Value::String(number.to_string())),
                Value::Bool(flag) => Some(Value::String(flag.to_string())),
                _ => None,
            },
            Self::Choice(choices) => {
                let given = value.as_str()?.trim();
                let exact = choices.iter().find(|choice| choice.as_str() == given);
                let folded = || {
                    let given = given.to_lowercase();
                    choices.iter().find(|choice| choice.to_lowercase() == given)
                };
                exact
                    .or_else(folded)
                    .map(|choice| Value::String(choice.clone()))
            }
            Self::Integer(range) => whole(range.keep(number(value)?).round()),
            Self::Number(range) => {
                let value = range.keep(number(value)?);
                let kept_whole = whole(value).filter(|_| value.fract() == 0.0);
                kept_whole.or_else(|| Number::from_f64(value).map(Value::Number))
            }
            Self::Flag => match value {
                Value::Bool(flag) => Some(Value::Bool(*flag)),
                Value::String(text) if text.trim().eq_ignore_ascii_case("true") => {
                    Some(Value::Bool(true))
                }
                Value::String(text) if text.trim().eq_ignore_ascii_case("false") => {
                    Some(Value::Bool(false))
                }
                _ => None,
            },
        }
    }
}

impl Range {
    fn keep(&self, value: f64) -> f64 {
        let value = self.least.map_or(value, |least| value.max(least));
        self.most.map_or(value, |most| value.min(most))
    }
}

fn given(raw: Option<Value>) -> Option<Map<String, Value>> {
    match raw {
        None | Some(Value::Null) => Some(Map::new()),
        Some(Value::Object(args)) => Some(args),
        Some(Value::Array(pairs)) => pairs
            .into_iter()
            .map(|pair| {
                let name = pair.get("name")?.as_str()?.trim().to_owned();
                Some((name, pair.get("value")?.clone()))
            })
            .collect(),
        Some(_) => None,
    }
}

fn number(value: &Value) -> Option<f64> {
    let number = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    };
    number.filter(|number| number.is_finite())
}

fn whole(value: f64) -> Option<Value> {
    (value.abs() < i64::MAX as f64).then(|| Value::from(value as i64))
}

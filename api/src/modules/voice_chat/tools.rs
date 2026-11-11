use std::collections::HashSet;

use llm_ladder::Schema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

const MAX_NAME_LEN: usize = 32;
const MAX_DESCRIPTION_CHARS: usize = 600;
const MAX_ARGS: usize = 16;
const MAX_CHOICES: usize = 32;
const MAX_CHOICE_CHARS: usize = 64;

#[derive(Deserialize)]
pub struct ToolSpec {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    args: Value,
    #[serde(default)]
    returns: bool,
}

#[derive(Default)]
pub struct Tools(Vec<Tool>);

pub struct Tool {
    name: String,
    description: String,
    args: Vec<Arg>,
    returns: bool,
}

struct Arg {
    name: String,
    kind: Kind,
    required: bool,
    description: String,
}

enum Kind {
    Text,
    Choice(Vec<String>),
    Integer(Range),
    Number(Range),
    Flag,
}

#[derive(Clone, Copy, Default)]
struct Range {
    least: Option<f64>,
    most: Option<f64>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Action {
    pub name: String,
    pub args: Map<String, Value>,
}

#[derive(Deserialize)]
struct ArgsSpec {
    #[serde(default)]
    properties: Map<String, Value>,
    #[serde(default)]
    required: Vec<String>,
}

#[derive(Deserialize)]
struct ArgSpec {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "enum")]
    choices: Option<Vec<String>>,
    #[serde(default)]
    description: String,
    minimum: Option<f64>,
    maximum: Option<f64>,
}

impl Tools {
    pub fn read(specs: Vec<ToolSpec>, most: usize) -> Result<Self, String> {
        if specs.len() > most {
            return Err(format!("tools has more than {most} items"));
        }
        let mut names = HashSet::new();
        let mut tools = Vec::with_capacity(specs.len());
        for spec in specs {
            if !is_name(&spec.name) {
                return Err(
                    "a tool name is not a lowercase identifier of up to 32 characters".to_owned(),
                );
            }
            if !names.insert(spec.name.clone()) {
                return Err(format!("tool {} is listed twice", spec.name));
            }
            let name = spec.name.clone();
            tools.push(Tool::read(spec).map_err(|reason| format!("tool {name}: {reason}"))?);
        }
        Ok(Self(tools))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn returns(&self, name: &str) -> bool {
        self.0.iter().any(|tool| tool.returns && tool.name == name)
    }

    pub fn without_returning(self) -> Self {
        Self(self.0.into_iter().filter(|tool| !tool.returns).collect())
    }

    pub fn answer_schema(&self) -> Schema {
        let say = ("say", Schema::String);
        if self.is_empty() {
            return Schema::object([say]);
        }
        let value = Schema::AnyOf(vec![Schema::String, Schema::Number, Schema::Boolean]);
        let argument = Schema::object([("name", Schema::String), ("value", value)]);
        let action = Schema::object([
            (
                "name",
                Schema::one_of(self.0.iter().map(|tool| tool.name.as_str())),
            ),
            ("args", Schema::array(argument)),
        ]);
        Schema::object([("actions", Schema::array(action)), say])
    }

    pub fn described(&self) -> impl Iterator<Item = String> + '_ {
        self.0.iter().map(Tool::described)
    }

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

    fn named(&self, name: &str) -> Option<&Tool> {
        let name = name.trim();
        let exact = self.0.iter().find(|tool| tool.name == name);
        exact.or_else(|| {
            self.0
                .iter()
                .find(|tool| tool.name.eq_ignore_ascii_case(name))
        })
    }
}

pub fn is_name(name: &str) -> bool {
    let mut symbols = name.chars();
    symbols
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
        && name.len() <= MAX_NAME_LEN
        && symbols
            .all(|symbol| symbol.is_ascii_lowercase() || symbol.is_ascii_digit() || symbol == '_')
}

impl Tool {
    fn read(spec: ToolSpec) -> Result<Self, String> {
        if spec.description.chars().count() > MAX_DESCRIPTION_CHARS {
            return Err(format!(
                "description is longer than {MAX_DESCRIPTION_CHARS} characters"
            ));
        }
        Ok(Self {
            name: spec.name,
            description: one_line(&spec.description),
            args: args(spec.args)?,
            returns: spec.returns,
        })
    }

    fn described(&self) -> String {
        let args: Vec<String> = self.args.iter().map(Arg::signature).collect();
        let mut line = format!("- {}({})", self.name, args.join(", "));
        if !self.description.is_empty() {
            line.push_str(": ");
            line.push_str(&self.description);
        }
        for arg in self.args.iter().filter(|arg| !arg.description.is_empty()) {
            line.push_str(&format!(" [{}: {}]", arg.name, arg.description));
        }
        if self.returns {
            line.push_str(" (returns a result)");
        }
        line
    }
}

impl Arg {
    fn read(name: String, spec: Value, required: bool) -> Result<Self, String> {
        if !is_name(&name) {
            return Err(
                "an argument name is not a lowercase identifier of up to 32 characters".to_owned(),
            );
        }
        let spec: ArgSpec = serde_json::from_value(spec)
            .map_err(|_| format!("argument {name} is not a primitive property"))?;
        if spec.description.chars().count() > MAX_DESCRIPTION_CHARS {
            return Err(format!(
                "argument {name} has a description longer than {MAX_DESCRIPTION_CHARS} characters"
            ));
        }
        let range = Range {
            least: spec.minimum.filter(|bound| bound.is_finite()),
            most: spec.maximum.filter(|bound| bound.is_finite()),
        };
        let kind = match (spec.kind.as_deref(), spec.choices) {
            (None | Some("string"), Some(choices)) => Kind::Choice(listed(&name, choices)?),
            (Some("string"), None) => Kind::Text,
            (Some("integer"), None) => Kind::Integer(range),
            (Some("number"), None) => Kind::Number(range),
            (Some("boolean"), None) => Kind::Flag,
            _ => {
                return Err(format!(
                    "argument {name} must be a string, number, integer, boolean or enum"
                ));
            }
        };
        Ok(Self {
            name,
            kind,
            required,
            description: one_line(&spec.description),
        })
    }

    fn left_out(&self, value: &Value) -> bool {
        match value {
            Value::Null => true,
            Value::String(text) => !self.required && text.trim().is_empty(),
            _ => false,
        }
    }

    fn signature(&self) -> String {
        let optional = if self.required { "" } else { "?" };
        format!("{}{optional}: {}", self.name, self.kind.described())
    }
}

impl Kind {
    fn described(&self) -> String {
        match self {
            Self::Text => "string".to_owned(),
            Self::Choice(choices) => {
                let quoted: Vec<String> = choices
                    .iter()
                    .map(|choice| format!("\"{choice}\""))
                    .collect();
                quoted.join(" | ")
            }
            Self::Integer(range) => format!("integer{}", range.described()),
            Self::Number(range) => format!("number{}", range.described()),
            Self::Flag => "boolean".to_owned(),
        }
    }

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

    fn described(&self) -> String {
        match (self.least, self.most) {
            (Some(least), Some(most)) => format!(" {least}..{most}"),
            (Some(least), None) => format!(" from {least}"),
            (None, Some(most)) => format!(" up to {most}"),
            (None, None) => String::new(),
        }
    }
}

fn args(spec: Value) -> Result<Vec<Arg>, String> {
    if spec.is_null() {
        return Ok(Vec::new());
    }
    let spec: ArgsSpec =
        serde_json::from_value(spec).map_err(|_| "args is not a flat object schema".to_owned())?;
    if spec.properties.len() > MAX_ARGS {
        return Err(format!("has more than {MAX_ARGS} arguments"));
    }
    if spec
        .required
        .iter()
        .any(|name| !spec.properties.contains_key(name))
    {
        return Err("requires an argument it does not declare".to_owned());
    }
    spec.properties
        .into_iter()
        .map(|(name, property)| {
            let required = spec.required.contains(&name);
            Arg::read(name, property, required)
        })
        .collect()
}

fn listed(name: &str, choices: Vec<String>) -> Result<Vec<String>, String> {
    let fits = !choices.is_empty()
        && choices.len() <= MAX_CHOICES
        && choices.iter().all(|choice| {
            let length = choice.chars().count();
            (1..=MAX_CHOICE_CHARS).contains(&length) && !choice.contains(['"', '\n', '\r'])
        });
    match fits {
        true => Ok(choices),
        false => Err(format!(
            "argument {name} lists no values, too many or too long ones"
        )),
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

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;

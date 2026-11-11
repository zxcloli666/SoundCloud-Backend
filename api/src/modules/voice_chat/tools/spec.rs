use std::collections::HashSet;

use serde::Deserialize;
use serde_json::{Map, Value};

use super::{Arg, Kind, Range, Tool, Tools, is_name};

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

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

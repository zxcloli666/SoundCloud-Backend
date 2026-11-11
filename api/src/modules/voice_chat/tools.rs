mod describe;
mod fit;
mod spec;

use llm_ladder::Schema;
use serde::Serialize;
use serde_json::{Map, Value};

pub use spec::ToolSpec;

const MAX_NAME_LEN: usize = 32;

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

impl Tools {
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

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;

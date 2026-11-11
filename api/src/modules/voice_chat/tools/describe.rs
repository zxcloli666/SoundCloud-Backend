use super::super::prompt::seal;
use super::{Arg, Kind, Range, Tool, Tools};

impl Tools {
    pub fn described(&self) -> impl Iterator<Item = String> + '_ {
        self.0.iter().map(Tool::described)
    }
}

impl Tool {
    fn described(&self) -> String {
        let args: Vec<String> = self.args.iter().map(Arg::signature).collect();
        let mut line = format!("- {}({})", self.name, args.join(", "));
        if !self.description.is_empty() {
            line.push_str(": ");
            line.push_str(&seal(&self.description));
        }
        for arg in self.args.iter().filter(|arg| !arg.description.is_empty()) {
            line.push_str(&format!(" [{}: {}]", arg.name, seal(&arg.description)));
        }
        if self.returns {
            line.push_str(" (returns a result)");
        }
        line
    }
}

impl Arg {
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
                    .map(|choice| format!("\"{}\"", seal(choice)))
                    .collect();
                quoted.join(" | ")
            }
            Self::Integer(range) => format!("integer{}", range.described()),
            Self::Number(range) => format!("number{}", range.described()),
            Self::Flag => "boolean".to_owned(),
        }
    }
}

impl Range {
    fn described(&self) -> String {
        match (self.least, self.most) {
            (Some(least), Some(most)) => format!(" {least}..{most}"),
            (Some(least), None) => format!(" from {least}"),
            (None, Some(most)) => format!(" up to {most}"),
            (None, None) => String::new(),
        }
    }
}

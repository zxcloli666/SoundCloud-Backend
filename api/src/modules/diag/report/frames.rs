use backend_contracts::diag_report::Frame;

pub fn parse(stack: &str) -> Vec<Frame> {
    stack.lines().filter_map(frame).collect()
}

fn frame(line: &str) -> Option<Frame> {
    let line = line.trim();
    let start = line.find("assets/")?;
    let location = line[start..]
        .split(|c: char| c == ')' || c.is_whitespace())
        .next()?;
    let mut parts = location.splitn(3, ':');
    let file = parts.next()?.to_owned();
    let line_number = parts.next()?.parse().ok()?;
    let column = parts.next()?.parse().ok()?;
    Some(Frame {
        function: function_name(line),
        file,
        line: line_number,
        column,
        source: None,
        source_line: None,
        source_column: None,
        symbol: None,
    })
}

fn function_name(line: &str) -> Option<String> {
    let name = if let Some(rest) = line.strip_prefix("at ") {
        rest.split_once(" (")?.0
    } else {
        line.split_once('@')?.0
    };
    Some(name.to_owned()).filter(|name| !name.is_empty() && !name.contains("://"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_and_webkit_lines_both_parse() {
        let stack = "Error: boom\n    at render (https://tauri.localhost/assets/index-AbC123.js:1:2345)\n    at https://tauri.localhost/assets/vendor-Z9.js:7:8\ngo@tauri://localhost/assets/index-AbC123.js:10:20\n@tauri://localhost/assets/x-1.js:3:4\nnot a frame";
        let frames = parse(stack);

        assert_eq!(frames.len(), 4);
        assert_eq!(frames[0].function.as_deref(), Some("render"));
        assert_eq!(frames[0].file, "assets/index-AbC123.js");
        assert_eq!((frames[0].line, frames[0].column), (1, 2345));
        assert_eq!(frames[1].function, None);
        assert_eq!(frames[2].function.as_deref(), Some("go"));
        assert_eq!(frames[3].function, None);
    }
}

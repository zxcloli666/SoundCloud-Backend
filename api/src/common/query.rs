pub fn parse_languages(raw: Option<&str>) -> Option<Vec<String>> {
    parse_list(raw)
}

pub fn parse_genres(raw: Option<&str>) -> Option<Vec<String>> {
    parse_list(raw)
}

fn parse_list(raw: Option<&str>) -> Option<Vec<String>> {
    let s = raw?;
    let v: Vec<String> = s
        .split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(String::from)
        .collect();
    if v.is_empty() { None } else { Some(v) }
}

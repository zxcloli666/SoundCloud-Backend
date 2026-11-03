pub struct PageFields<'a> {
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub state: &'a str,
    pub scope: &'a str,
    pub assistant: &'a str,
    pub target: &'a str,
}

pub fn escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

pub fn form(fields: &PageFields<'_>, error: Option<&str>) -> String {
    let error_html = match error {
        Some(text) => format!("<p class=\"error\" role=\"alert\">{}</p>", escape(text)),
        None => String::new(),
    };
    FORM_TEMPLATE
        .replace("__CLIENT_ID__", &escape(fields.client_id))
        .replace("__REDIRECT_URI__", &escape(fields.redirect_uri))
        .replace("__STATE__", &escape(fields.state))
        .replace("__SCOPE__", &escape(fields.scope))
        .replace("__ASSISTANT__", &escape(fields.assistant))
        .replace("__TARGET__", &escape(fields.target))
        .replace("__ERROR__", &error_html)
        .replace("__STYLE__", STYLE)
}

pub fn failure(text: &str) -> String {
    FAILURE_TEMPLATE
        .replace("__MESSAGE__", &escape(text))
        .replace("__STYLE__", STYLE)
}

const STYLE: &str = r#"
    :root { color-scheme: light dark; --bg: #f6f6f8; --card: #ffffff; --text: #16161a; --muted: #6b6b76; --line: #dcdce3; --accent: #ff5500; --accent-text: #ffffff; --error: #c0262d; }
    @media (prefers-color-scheme: dark) {
      :root { --bg: #0e0e12; --card: #18181f; --text: #f2f2f5; --muted: #9a9aa6; --line: #2c2c36; --accent: #ff7b3a; --accent-text: #1a0e05; --error: #ff7a80; }
    }
    * { box-sizing: border-box; margin: 0; padding: 0; }
    html, body { min-height: 100%; }
    body { background: var(--bg); color: var(--text); font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; display: flex; align-items: center; justify-content: center; padding: 16px; }
    main { width: 100%; max-width: 380px; background: var(--card); border: 1px solid var(--line); border-radius: 20px; padding: 28px 22px; }
    h1 { font-size: 22px; font-weight: 650; margin-bottom: 8px; }
    p { color: var(--muted); font-size: 15px; line-height: 1.45; }
    form { margin-top: 20px; display: flex; flex-direction: column; gap: 14px; }
    input[type=text] { width: 100%; font-size: 32px; letter-spacing: 0.3em; text-align: center; padding: 14px 8px; border-radius: 14px; border: 1px solid var(--line); background: transparent; color: var(--text); font-variant-numeric: tabular-nums; }
    input[type=text]:focus { outline: 2px solid var(--accent); outline-offset: 1px; }
    button { font-size: 17px; font-weight: 600; padding: 14px; border: 0; border-radius: 14px; background: var(--accent); color: var(--accent-text); cursor: pointer; }
    .error { color: var(--error); margin-top: 14px; font-size: 15px; }
"#;

const FORM_TEMPLATE: &str = r#"<!DOCTYPE html>
<html lang="ru">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<title>Связать __ASSISTANT__</title>
<style>__STYLE__</style>
</head>
<body>
<main>
<h1>Связать __ASSISTANT__</h1>
<p>Введите шесть цифр из приложения на компьютере: Голос, Колонки, Подключить __TARGET__.</p>
__ERROR__
<form method="post" action="/oauth/authorize">
<input type="hidden" name="client_id" value="__CLIENT_ID__">
<input type="hidden" name="redirect_uri" value="__REDIRECT_URI__">
<input type="hidden" name="state" value="__STATE__">
<input type="hidden" name="scope" value="__SCOPE__">
<input type="text" name="code" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9 ]*" maxlength="8" placeholder="000000" autofocus required>
<button type="submit">Связать</button>
</form>
</main>
</body>
</html>
"#;

const FAILURE_TEMPLATE: &str = r#"<!DOCTYPE html>
<html lang="ru">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Не получилось</title>
<style>__STYLE__</style>
</head>
<body>
<main>
<h1>Не получилось</h1>
<p>__MESSAGE__</p>
</main>
</body>
</html>
"#;

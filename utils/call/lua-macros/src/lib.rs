use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, LitStr};

const DENYLIST: &[&str] = &[
    "os",
    "io",
    "package",
    "debug",
    "require",
    "load",
    "loadstring",
    "dofile",
    "loadfile",
    "collectgarbage",
    "_G",
    "_ENV",
    "getmetatable",
    "setmetatable",
    "rawget",
    "rawset",
    "rawequal",
    "rawlen",
    "newproxy",
];

#[proc_macro]
pub fn lua_script(input: TokenStream) -> TokenStream {
    let path_lit = parse_macro_input!(input as LitStr);
    let rel = path_lit.value();

    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let abs = std::path::Path::new(&manifest).join(&rel);
    let abs_str = abs.to_string_lossy().into_owned();

    let src = match std::fs::read_to_string(&abs) {
        Ok(s) => s,
        Err(e) => return err(&path_lit, format!("lua_script: cannot read {abs_str}: {e}")),
    };

    if let Err(errs) = full_moon::parse(&src) {
        let msg = errs
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        return err(&path_lit, format!("lua_script: {rel}: syntax error: {msg}"));
    }

    if let Some(bad) = scan_denylist(&src) {
        return err(
            &path_lit,
            format!(
                "lua_script: {rel}: forbidden identifier `{bad}` — \
                 os/io/package/debug/require/load*/raw*/metatable escapes are banned \
                 in sandboxed methods"
            ),
        );
    }

    let src_lit = src.as_str();
    quote! {
        {
            const _: &[u8] = include_bytes!(#abs_str);
            #src_lit
        }
    }
    .into()
}

fn scan_denylist(src: &str) -> Option<String> {
    use full_moon::tokenizer::{Lexer, LexerResult, TokenType};

    let mut lexer = Lexer::new(src, full_moon::LuaVersion::default());
    while let Some(res) = lexer.process_next() {
        let token = match res {
            LexerResult::Ok(t) | LexerResult::Recovered(t, _) => t,
            LexerResult::Fatal(_) => break,
        };
        if let TokenType::Identifier { identifier } = token.token_type() {
            let name = identifier.as_str();
            if DENYLIST.contains(&name) {
                return Some(name.to_string());
            }
        }
    }
    None
}

fn err(spanned: &LitStr, msg: String) -> TokenStream {
    syn::Error::new(spanned.span(), msg)
        .to_compile_error()
        .into()
}

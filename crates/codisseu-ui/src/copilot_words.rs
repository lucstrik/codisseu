//! Partial-accept boundaries follow VS Code 1.145.0 language configurations.
//! Definitions: Copyright (c) Microsoft Corporation, MIT; notice and pinned
//! sources in reference/public/vscode-inline and reference/copilot-inline-reference-20261106.md.
use regex::Regex;
use std::sync::OnceLock;

pub(super) fn next_word(text: &str, language: &str) -> usize {
    static RULES: OnceLock<([Regex; 4], Regex)> = OnceLock::new();
    let (rules,spaces)=RULES.get_or_init(|| {
        // ECMAScript whitespace, including BOM and excluding U+0085.
        let space=r"\x09-\x0d\x20\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}\x{feff}";
        let separators="`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";
        let build=|allowed:&str| {
            let excluded: String=separators.chars().filter(|ch|!allowed.contains(*ch)).collect();
            Regex::new(&format!(r"(-?[0-9]*\.[0-9][A-Za-z0-9_]*)|([^{}{space}]+)",regex::escape(&excluded))).expect("versioned word definition")
        };
        let base=r"[\p{Alphabetic}\p{Number}\p{Nonspacing_Mark}]";
        let markdown=Regex::new(&format!("{base}(({base}|[_])?{base})*")).expect("versioned Markdown word definition");
        ([build(""),build("$"),build("$#"),markdown],Regex::new(&format!("[{space}]+")).expect("ECMAScript whitespace"))
    });
    let rule = match language {
        "php" => 1,
        "javascript" | "javascriptreact" | "typescript" | "typescriptreact" => 2,
        "markdown" => 3,
        _ => 0,
    };
    let mut limit =
        rules[rule].find(text).map_or(
            text.len(),
            |m| if m.start() == 0 { m.end() } else { m.start() },
        );
    if let Some(space) = spaces.find(text)
        && space.end() < limit
    {
        limit = space.end();
    }
    text[..limit].chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copilot_partial_word_agrees_with_102_pinned_reference_regexp_cases() {
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/copilot_words.json")).unwrap();
        let cases = data["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 102);
        for case in cases {
            let language = match case["language"].as_str().unwrap() {
                "typescript-basics" => "typescript",
                "markdown-basics" => "markdown",
                other => other,
            };
            let text = case["text"].as_str().unwrap();
            let accepted: String = text.chars().take(next_word(text, language)).collect();
            assert_eq!(
                accepted,
                case["accepted"].as_str().unwrap(),
                "{language}: {text}"
            );
            assert_eq!(
                accepted.encode_utf16().count() as u64,
                case["utf16"].as_u64().unwrap()
            );
        }
    }
    #[test]
    fn copilot_partial_word_matches_reference_punctuation_numbers_and_language_rules() {
        for (text, language, expected) in [
            ("const result", "typescript", "const"),
            (" result", "typescript", " "),
            ("); return", "javascript", "); "),
            ("-12.5f next", "sql", "-12.5f"),
            ("-12 next", "sql", "-"),
            ("$value rest", "php", "$value"),
            ("$value rest", "sql", "$"),
            ("#private rest", "typescript", "#private"),
            ("😀ação rest", "javascript", "😀ação"),
            ("e\u{301} rest", "markdown", "e\u{301}"),
            ("a_b rest", "markdown", "a_b"),
            ("a__b rest", "markdown", "a"),
            ("\u{feff}next", "plaintext", "\u{feff}"),
        ] {
            let n = next_word(text, language);
            assert_eq!(
                text.chars().take(n).collect::<String>(),
                expected,
                "{language}: {text}"
            );
        }
    }
}

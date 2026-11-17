use codisseu_syntax::{
    RenameOccurrenceKind, TextRange, ValidationLimits, rename_occurrence_presentation,
    search_symbols::{self, Kind},
};
use std::{path::Path, sync::atomic::AtomicBool};
#[test]
fn python_classes_methods_async_decorators_docstrings_unicode_and_presentation_are_syntactic() {
    let source = "# def phantom() 😀\r\nclass Forecast:\r\n    \"\"\"def phantom(): literal\"\"\"\r\n    @property\r\n    def ação(self) -> int:\r\n        return 42\r\nasync def run():\r\n    return Forecast()\r\n";
    for extension in ["py", "pyi", "pyw"] {
        let path = format!("main.{extension}");
        let out = search_symbols::declarations(Path::new(&path), source, &|| false);
        assert!(!out.truncated);
        assert_eq!(out.symbols.len(), 3, "{out:?}");
        assert!(
            out.symbols
                .iter()
                .any(|s| s.name == "Forecast" && s.kind == Kind::Class)
        );
        assert!(
            out.symbols
                .iter()
                .any(|s| s.name == "ação" && s.kind == Kind::Method)
        );
        assert!(
            out.symbols
                .iter()
                .any(|s| s.name == "run" && s.kind == Kind::Function)
        );
        let at = source.find("ação").unwrap();
        let prefix = &source[..at];
        let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32;
        let col = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
        let out = rename_occurrence_presentation(
            Path::new(&path),
            source,
            &[TextRange {
                start_line: line,
                start_character: col,
                end_line: line,
                end_character: col + 4,
            }],
            &AtomicBool::new(false),
            ValidationLimits::default(),
        );
        assert_eq!(out[0].kind, RenameOccurrenceKind::Declaration);
        assert!(out[0].unknown_reason.is_none());
        assert!(out[0].enclosing.iter().any(|s| s.name == "Forecast"));
    }
    assert!(search_symbols::declarations(Path::new("main.py"), source, &|| true).truncated);
}

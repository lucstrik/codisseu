#[test]
fn executable_prints_unicode_and_literal_arguments() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_codisseu-fixture-app"))
        .args(["ação 🦀", "spaces ; $(literal) `literal` ' \""])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Olá, Codisseu 🦀! total=42"));
    assert!(text.contains("arg[0]=ação 🦀"));
    assert!(text.contains("arg[1]=spaces ; $(literal) `literal` ' \""));
}

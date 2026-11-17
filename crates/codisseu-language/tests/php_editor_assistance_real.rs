//! Cross-file PHP assistance over the actual configured provider, never a test peer.
use codisseu_language::{
    Language, LanguageCommand, LanguageService, ProviderConfig, ProviderState, SignatureHelpTrigger,
};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn wait(service: &mut LanguageService, check: impl Fn(&LanguageService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        service.poll();
        if check(service) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "statuses={:?}, last={:?}, hover={:?}, signature={:?}, completion={:?}",
            service.statuses,
            service.last_error,
            service.hover_error,
            service.signature_help_error,
            service.completion_error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn position(text: &str, byte: usize) -> (u32, u32) {
    let prefix = &text[..byte];
    (
        prefix.bytes().filter(|byte| *byte == b'\n').count() as u32,
        prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32,
    )
}

#[test]
#[ignore = "actual Phpactor; requires explicit CODISSEU_PROVIDER_CONFIG"]
fn real_php_cross_file_method_hover_completion_signature_and_unsaved_phpdoc() {
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(
            std::env::var_os("CODISSEU_PROVIDER_CONFIG")
                .expect("explicit provider configuration required"),
        )
        .unwrap(),
    )
    .unwrap();
    let php = configs
        .into_iter()
        .filter(|config| config.language == Language::Php)
        .collect::<Vec<_>>();
    assert_eq!(php.len(), 1);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        root.join("composer.json"),
        r#"{"name":"codisseu/php-assistance-fixture","autoload":{"psr-4":{"App\\":"src/"}}}"#,
    )
    .unwrap();
    let declaration = root.join("src/Greeting.php");
    let caller = root.join("src/Consumer.php");
    let method = "<?php\nnamespace App;\nclass Greeting {\n    /** Greeting for the selected customer.\n     * @param string $name Customer name.\n     * @param int $repeat Repeat count.\n     * @return string Formatted greeting.\n     */\n    public function greet(string $name, int $repeat = 1): string {\n        return str_repeat('Hello ' . $name, $repeat);\n    }\n}\n";
    let main = "<?php\nnamespace App;\nclass Consumer {\n    public function run(Greeting $greeter): string {\n        /* 😀 e\u{301} */ return $greeter->greet('Ada', 2);\n    }\n}\n";
    fs::write(&declaration, method).unwrap();
    fs::write(&caller, main).unwrap();
    let mut service = LanguageService::new();
    service
        .dispatch(LanguageCommand::Start {
            workspace: root,
            configs: php,
        })
        .unwrap();
    service
        .dispatch(LanguageCommand::Sync {
            path: caller.clone(),
            version: 1,
            text: main.into(),
        })
        .unwrap();
    wait(&mut service, |service| {
        service
            .statuses
            .iter()
            .any(|status| status.state == ProviderState::Ready)
            && !service.is_busy()
    });
    let (line, character) = position(main, main.find("greet('").unwrap());
    service
        .dispatch(LanguageCommand::Hover {
            path: caller.clone(),
            version: 1,
            line,
            character,
        })
        .unwrap();
    wait(&mut service, |service| !service.hover_pending());
    assert!(service.hover_error.is_none(), "{:?}", service.hover_error);
    let hover = service.hover.as_ref().expect("actual cross-file hover");
    assert!(
        hover.text.contains("greet") && hover.text.contains("Greeting for the selected customer"),
        "{}",
        hover.text
    );
    assert!(
        hover.text.contains("$name") && hover.text.contains("string"),
        "{}",
        hover.text
    );

    let partial = main.replace("greet('Ada', 2)", "gr");
    service
        .dispatch(LanguageCommand::Sync {
            path: caller.clone(),
            version: 2,
            text: partial.clone(),
        })
        .unwrap();
    let (line, character) = position(&partial, partial.find("->gr").unwrap() + 4);
    service
        .dispatch(LanguageCommand::Completion {
            path: caller.clone(),
            version: 2,
            line,
            character,
        })
        .unwrap();
    wait(&mut service, |service| !service.completion_pending());
    assert!(
        service.completion_error.is_none(),
        "{:?}",
        service.completion_error
    );
    let completion = service
        .completion
        .as_ref()
        .unwrap()
        .items
        .iter()
        .find(|item| item.label.starts_with("greet"))
        .expect("greet method from another file");
    assert_eq!(completion.kind, Some(2));
    assert!(
        completion.unsupported_reason.is_none(),
        "{:?}",
        completion.unsupported_reason
    );

    service
        .dispatch(LanguageCommand::Sync {
            path: caller.clone(),
            version: 3,
            text: main.into(),
        })
        .unwrap();
    let (line, character) = position(main, main.find("greet('").unwrap() + "greet(".len());
    service
        .dispatch(LanguageCommand::SignatureHelp {
            path: caller.clone(),
            version: 3,
            line,
            character,
            trigger: SignatureHelpTrigger::Invoked,
            is_retrigger: false,
        })
        .unwrap();
    wait(&mut service, |service| !service.signature_help_pending());
    assert!(
        service.signature_help_error.is_none(),
        "{:?}",
        service.signature_help_error
    );
    let signature = service.signature_help.as_ref().unwrap();
    assert!(!signature.signatures.is_empty());
    let signature = &signature.signatures[0];
    assert!(
        signature.label.contains("$name") && signature.label.contains("$repeat"),
        "{}",
        signature.label
    );
    assert_eq!(signature.parameters.len(), 2);

    let updated = method.replace(
        "Greeting for the selected customer",
        "Updated greeting from the unsaved declaration",
    );
    service
        .dispatch(LanguageCommand::Sync {
            path: declaration.clone(),
            version: 1,
            text: method.into(),
        })
        .unwrap();
    service
        .dispatch(LanguageCommand::Sync {
            path: declaration.clone(),
            version: 2,
            text: updated,
        })
        .unwrap();
    let (line, character) = position(main, main.find("greet('").unwrap());
    service
        .dispatch(LanguageCommand::Hover {
            path: caller.clone(),
            version: 3,
            line,
            character,
        })
        .unwrap();
    wait(&mut service, |service| !service.hover_pending());
    assert!(service.hover_error.is_none(), "{:?}", service.hover_error);
    assert!(
        service
            .hover
            .as_ref()
            .unwrap()
            .text
            .contains("Updated greeting from the unsaved declaration"),
        "{}",
        service.hover.as_ref().unwrap().text
    );
    service.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut service, |service| !service.is_busy());
    assert_eq!(fs::read_to_string(declaration).unwrap(), method);
    assert_eq!(fs::read_to_string(caller).unwrap(), main);
}

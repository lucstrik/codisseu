//! Explicit live probe: only a caller-provided disposable directory, no credential output.
use codisseu_ai::{
    Action, ChatService, Command, Config, Mode, State, attachments::AttachmentInput,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
fn wait(chat: &mut ChatService) {
    let start = Instant::now();
    while chat.is_running() {
        assert!(start.elapsed() < Duration::from_secs(150), "Live timeout");
        chat.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(chat.state, State::Completed, "{:?}", chat.error);
}
fn main() {
    let cwd = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("Pass disposable workspace"),
    )
    .canonicalize()
    .expect("Disposable workspace must exist");
    assert!(cwd.is_dir());
    let context = cwd.join("probe-context.txt");
    assert!(!context.exists(), "Use a fresh fixture");
    let context_bytes = b"Context token: contexto-azul-571\n";
    std::fs::write(&context, context_bytes).unwrap();
    let mut chat = ChatService::default();
    let config = Config {
        mode: Mode::Consult,
        ..Config::default()
    };
    chat.dispatch(
        Command::Start {
            config: config.clone(),
            action: Action::Check,
        },
        Some(cwd.clone()),
    )
    .unwrap();
    wait(&mut chat);
    assert_eq!(chat.authenticated, Some(true));
    println!("account/read: authenticated (no account data exported)");
    chat.dispatch(Command::Start{config:config.clone(),action:Action::SendWithAttachments {
        text: "Responda CODEX-CODISSEU-OK e o token do arquivo anexado. Memorize a palavra farol-roxo-829 para a próxima mensagem. Não use ferramentas nem altere arquivos.".into(),
        display_text: "Validar resposta, contexto anexado e memória".into(),
        attachments: vec![AttachmentInput::ProjectFile("probe-context.txt".into())],
    }},Some(cwd.clone())).unwrap();
    wait(&mut chat);
    assert!(
        chat.entries
            .iter()
            .any(|e| e.role == "Assistente" && e.text.contains("CODEX-CODISSEU-OK"))
    );
    assert!(
        chat.entries
            .iter()
            .any(|entry| entry.role == "Assistente" && entry.text.contains("contexto-azul-571"))
    );
    let session = chat.session_id().unwrap().to_owned();
    println!(
        "first real read-only turn: expected answer and attached context received; session allocated"
    );
    chat.dispatch(
        Command::Start {
            config: config.clone(),
            action: Action::Send(
                "Qual palavra pedi para memorizar? Responda só a palavra; sem ferramentas.".into(),
            ),
        },
        Some(cwd.clone()),
    )
    .unwrap();
    wait(&mut chat);
    assert_eq!(chat.session_id(), Some(session.as_str()));
    assert!(
        chat.entries
            .iter()
            .rev()
            .find(|e| e.role == "Assistente")
            .unwrap()
            .text
            .contains("farol-roxo-829")
    );
    println!("second real turn: explicit session resume preserved conversation");
    let target = cwd.join("agent-result.txt");
    assert!(!target.exists(), "Use a fresh fixture");
    let mut config = config;
    config.mode = codisseu_ai::Mode::Agent;
    chat.dispatch(Command::Start{config,action:Action::Send("Crie somente o arquivo agent-result.txt nesta pasta com o conteúdo exato CODISSEU-AGENT-OK seguido de newline. Use apply_patch. Não leia nem altere outros arquivos, não execute comandos de shell.".into())},Some(cwd.clone())).unwrap();
    let start = Instant::now();
    while chat.is_running() {
        assert!(start.elapsed() < Duration::from_secs(150));
        chat.poll();
        assert!(
            chat.approvals.is_empty(),
            "Unexpected permission escalation in workspace fixture"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(chat.state, State::Completed, "{:?}", chat.error);
    assert_eq!(
        std::fs::read_to_string(target).unwrap(),
        "CODISSEU-AGENT-OK\n"
    );
    assert!(
        chat.entries
            .iter()
            .any(|e| e.role == "Ferramenta" && e.text.starts_with("Arquivos"))
    );
    println!("real Agent turn: exact fixture file created; fileChange item received");
    assert_eq!(std::fs::read(context).unwrap(), context_bytes);
    let id = chat.active_chat_id().unwrap().to_owned();
    chat.dispatch(Command::DeleteChat(id), None).unwrap();
    let start = Instant::now();
    while chat.is_running() {
        assert!(start.elapsed() < Duration::from_secs(150), "Delete timeout");
        chat.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_ne!(chat.state, State::Failed, "{:?}", chat.error);
    assert!(chat.conversations().is_empty());
    assert_eq!(chat.session_id(), None);
    println!("thread/delete: acknowledged; synthetic conversation removed");
}

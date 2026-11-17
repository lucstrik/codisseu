use codisseu_ai::{ChatService, Command, Config, Mode, Provider};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn settled(chat: &mut ChatService) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        chat.poll();
        if chat.profile_ready() && !chat.profile_pending() {
            assert!(chat.profile_error.is_none(), "{:?}", chat.profile_error);
            return;
        }
        assert!(Instant::now() < deadline, "profile worker did not settle");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn configure(chat: &mut ChatService, provider: Provider, mode: Mode) {
    chat.dispatch(
        Command::Configure(Config {
            provider,
            program: match provider {
                Provider::Codex => "codex",
                Provider::Claude => "claude",
            }
            .into(),
            mode,
            ..Config::default()
        }),
        None,
    )
    .unwrap();
}

#[test]
fn explicit_defaults_survive_empty_history_restart_and_remain_project_and_provider_scoped() {
    let profile = tempfile::tempdir().unwrap();
    let path = profile.path().join("ai.json");
    let root = Path::new("/fixture/preferred-project");
    let other = Path::new("/fixture/other-project");
    let mut chat = ChatService::new(path.clone());
    settled(&mut chat);
    chat.sync_workspace(Some(root));
    configure(&mut chat, Provider::Codex, Mode::FullAccess);
    assert!(chat.preferred_config(Provider::Claude).is_none());
    configure(&mut chat, Provider::Claude, Mode::Consult);
    assert_eq!(
        chat.preferred_config(Provider::Codex).unwrap().mode,
        Mode::FullAccess
    );
    assert_eq!(
        chat.preferred_config(Provider::Claude).unwrap().mode,
        Mode::Consult
    );
    // Deleting transcripts does not silently discard a preference.
    for (id, _) in chat.conversations() {
        chat.dispatch(Command::DeleteChat(id), None).unwrap();
    }
    assert!(chat.conversations().is_empty());
    assert_eq!(chat.config.provider, Provider::Claude);
    assert_eq!(chat.config.mode, Mode::Consult);
    settled(&mut chat);
    drop(chat);

    let mut chat = ChatService::new(path);
    chat.sync_workspace(Some(root));
    settled(&mut chat);
    assert!(chat.conversations().is_empty());
    assert_eq!(chat.config.provider, Provider::Claude);
    assert_eq!(chat.config.mode, Mode::Consult);
    assert_eq!(
        chat.preferred_config(Provider::Codex).unwrap().mode,
        Mode::FullAccess
    );
    chat.sync_workspace(Some(other));
    assert_eq!(chat.config, Config::default());
    assert!(chat.preferred_config(Provider::Codex).is_none());
    assert!(chat.preferred_config(Provider::Claude).is_none());
    configure(&mut chat, Provider::Codex, Mode::Agent);
    chat.sync_workspace(Some(root));
    assert_eq!(
        chat.preferred_config(Provider::Codex).unwrap().mode,
        Mode::FullAccess
    );
    assert_eq!(chat.config.provider, Provider::Claude);
    settled(&mut chat);
    assert!(
        !chat.any_running(),
        "changing preferences must not start any CLI"
    );
}

#[test]
fn selecting_a_historical_permission_does_not_replace_the_explicit_new_chat_default() {
    let mut chat = ChatService::default();
    chat.sync_workspace(Some(Path::new("/fixture/project")));
    configure(&mut chat, Provider::Codex, Mode::Consult);
    let historical = chat.active_chat_id().unwrap().to_owned();
    chat.dispatch(Command::NewConversation, None).unwrap();
    configure(&mut chat, Provider::Codex, Mode::FullAccess);
    let full = chat.active_chat_id().unwrap().to_owned();
    chat.dispatch(Command::SelectChat(historical.clone()), None)
        .unwrap();
    assert_eq!(chat.config.mode, Mode::Consult);
    assert_eq!(
        chat.preferred_config(Provider::Codex).unwrap().mode,
        Mode::FullAccess
    );
    chat.dispatch(Command::NewConversation, None).unwrap();
    assert_eq!(chat.config.mode, Mode::FullAccess);
    chat.dispatch(Command::SelectChat(historical), None)
        .unwrap();
    assert_eq!(chat.config.mode, Mode::Consult);
    chat.dispatch(Command::SelectChat(full), None).unwrap();
    assert_eq!(chat.config.mode, Mode::FullAccess);
    assert!(!chat.any_running());
}

#[test]
fn schema_one_without_defaults_keeps_historical_mode_without_granting_other_providers() {
    let profile = tempfile::tempdir().unwrap();
    let path = profile.path().join("legacy.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "schema_version":1,
            "workspaces": {"/fixture/legacy": {
                "active":"legacy-chat",
                "chats":[{
                    "id":"legacy-chat", "title":"Legacy", "provider":"codex", "program":"codex",
                    "model":"", "effort":"", "mode":"full", "session":null, "entries":[]
                }]
            }}
        }))
        .unwrap(),
    )
    .unwrap();
    let mut chat = ChatService::new(path);
    chat.sync_workspace(Some(Path::new("/fixture/legacy")));
    settled(&mut chat);
    assert_eq!(chat.config.mode, Mode::FullAccess);
    assert!(chat.preferred_config(Provider::Codex).is_none());
    assert!(chat.preferred_config(Provider::Claude).is_none());
    chat.sync_workspace(Some(Path::new("/fixture/unrelated")));
    assert_eq!(chat.config.mode, Mode::Agent);
}

#[test]
fn parked_owner_checkpoint_persists_latest_letters_without_changing_selected_chat() {
    let profile = tempfile::tempdir().unwrap();
    let path = profile.path().join("checkpoint.json");
    let root_a = Path::new("/fixture/checkpoint-a");
    let root_b = Path::new("/fixture/checkpoint-b");
    let mut chat = ChatService::new(path.clone());
    settled(&mut chat);
    chat.sync_workspace(Some(root_a));
    configure(&mut chat, Provider::Codex, Mode::Consult);
    let a = chat.active_chat_id().unwrap().to_owned();
    chat.set_draft("antes da leitura assíncrona").unwrap();
    let at_a = chat.chat_updated_at(&a);
    chat.sync_workspace(Some(root_b));
    configure(&mut chat, Provider::Codex, Mode::FullAccess);
    let b = chat.active_chat_id().unwrap().to_owned();
    chat.set_draft("rascunho B independente").unwrap();
    let config_b = chat.config.clone();
    let at_b = chat.chat_updated_at(&b);
    chat.state = codisseu_ai::State::Failed;
    chat.error = Some("estado B preservado".into());
    chat.checkpoint_draft(root_a, &a, "últimas letras A depois de abrir B 🌧")
        .unwrap();
    assert_eq!(chat.workspace(), Some(root_b));
    assert_eq!(chat.active_chat_id(), Some(b.as_str()));
    assert_eq!(chat.config, config_b);
    assert_eq!(chat.state, codisseu_ai::State::Failed);
    assert_eq!(chat.error.as_deref(), Some("estado B preservado"));
    assert_eq!(chat.saved_draft(), Some("rascunho B independente"));
    assert_eq!(chat.chat_updated_at(&b), at_b);
    assert!(!chat.any_running());
    settled(&mut chat);
    chat.checkpoint_draft(root_a, &a, "últimas letras A depois de abrir B 🌧")
        .unwrap();
    assert!(
        !chat.profile_pending(),
        "same checkpoint must not schedule I/O"
    );
    drop(chat);

    let mut reopened = ChatService::new(path);
    reopened.sync_workspace(Some(root_b));
    settled(&mut reopened);
    assert_eq!(reopened.active_chat_id(), Some(b.as_str()));
    assert_eq!(reopened.config, config_b);
    assert_eq!(reopened.saved_draft(), Some("rascunho B independente"));
    reopened.sync_workspace(Some(root_a));
    assert_eq!(reopened.active_chat_id(), Some(a.as_str()));
    assert_eq!(reopened.config.mode, Mode::Consult);
    assert_eq!(
        reopened.saved_draft(),
        Some("últimas letras A depois de abrir B 🌧")
    );
    assert_eq!(reopened.chat_updated_at(&a), at_a);
}

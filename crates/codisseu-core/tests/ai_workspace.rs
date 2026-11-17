use codisseu_core::{AppService, Command, ai};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn wait(app: &mut AppService, done: impl Fn(&AppService) -> bool) {
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        app.poll();
        if done(app) {
            return;
        }
        assert!(Instant::now() < until, "{} {:?}", app.status, app.ai.error);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn send(app: &mut AppService, command: Command) {
    app.dispatch(command).unwrap();
    wait(app, |app| !app.is_busy() && app.ai.profile_ready());
    assert!(app.last_error.is_none(), "{:?}", app.last_error);
}
fn workspace(file: &Path, folders: &[PathBuf]) {
    fs::write(
        file,
        serde_json::to_vec(&serde_json::json!({
            "folders":folders.iter().map(|root|serde_json::json!({"path":root})).collect::<Vec<_>>()
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn ai_uses_declared_root_and_keeps_history_distinct_for_neighboring_workspace_files() {
    let dir = tempfile::tempdir().unwrap();
    let carrier = dir.path().join("carrier");
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    for root in [&carrier, &first, &second] {
        fs::create_dir(root).unwrap();
    }
    let one = carrier.join("one.code-workspace");
    let two = carrier.join("two.code-workspace");
    workspace(&one, std::slice::from_ref(&first));
    workspace(&two, std::slice::from_ref(&second));
    let mut app = AppService::new(dir.path().join("session.json"));
    send(&mut app, Command::OpenWorkspace(one.clone()));
    assert_eq!(app.workspace, Some(carrier.clone()));
    assert_eq!(app.ai_workspace(), Some(first.as_path()));
    assert_eq!(app.ai.workspace(), Some(first.as_path()));
    send(&mut app, Command::Ai(ai::Command::NewConversation));
    let first_chat = app.ai.active_chat_id().unwrap().to_owned();
    app.ai.set_draft("rascunho da primeira pasta 🦀").unwrap();
    send(&mut app, Command::OpenWorkspace(two));
    assert_eq!(app.ai_workspace(), Some(second.as_path()));
    assert!(app.ai.conversations().is_empty());
    send(&mut app, Command::Ai(ai::Command::NewConversation));
    assert_ne!(app.ai.active_chat_id(), Some(first_chat.as_str()));
    app.ai.set_draft("rascunho da segunda pasta").unwrap();
    send(&mut app, Command::OpenWorkspace(one));
    assert_eq!(app.ai.active_chat_id(), Some(first_chat.as_str()));
    assert_eq!(app.ai.saved_draft(), Some("rascunho da primeira pasta 🦀"));
    assert_eq!(app.ai.conversations().len(), 1);
    assert_ne!(app.ai.workspace(), Some(carrier.as_path()));
}

#[test]
fn active_declared_folder_wins_but_external_and_nested_projects_do_not_expand_scope() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    let nested = second.join("nested");
    let absent = dir.path().join("absent");
    fs::create_dir(&first).unwrap();
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("package.json"), "{}").unwrap();
    fs::write(nested.join("note.txt"), "nested bytes").unwrap();
    let external = dir.path().join("outside.txt");
    fs::write(&external, "outside bytes").unwrap();
    let file = dir.path().join("team.code-workspace");
    workspace(&file, &[absent, first.clone(), second.clone()]);
    let mut app = AppService::new(dir.path().join("session.json"));
    send(&mut app, Command::OpenWorkspace(file));
    assert_eq!(app.ai_workspace(), Some(first.as_path()));
    send(&mut app, Command::OpenFile(nested.join("note.txt")));
    assert_eq!(app.active_document().unwrap().project_root(), nested);
    assert_eq!(app.ai_workspace(), Some(second.as_path()));
    assert_eq!(app.ai.workspace(), Some(second.as_path()));
    send(&mut app, Command::Ai(ai::Command::NewConversation));
    app.ai.set_draft("segunda pasta").unwrap();
    let second_chat = app.ai.active_chat_id().unwrap().to_owned();
    // A declared root outside the workspace carrier remains a valid @ source.
    let attachments = ai::attachments::prepare(
        app.ai_workspace().unwrap(),
        &[ai::attachments::AttachmentInput::ProjectFile(
            nested.join("note.txt"),
        )],
    )
    .unwrap();
    assert!(attachments.text_context.contains("nested bytes"));
    assert!(
        ai::attachments::prepare(
            app.ai_workspace().unwrap(),
            &[ai::attachments::AttachmentInput::ProjectFile(
                external.clone()
            )],
        )
        .is_err()
    );
    send(&mut app, Command::OpenFile(external));
    assert_eq!(app.ai_workspace(), Some(first.as_path()));
    assert!(app.ai.conversations().is_empty());
    send(&mut app, Command::OpenFile(nested.join("note.txt")));
    assert_eq!(app.ai.active_chat_id(), Some(second_chat.as_str()));
    assert_eq!(app.ai.saved_draft(), Some("segunda pasta"));
    send(&mut app, Command::OpenWorkspace(second.clone()));
    send(&mut app, Command::OpenFile(nested.join("note.txt")));
    assert_eq!(app.ai_workspace(), Some(second.as_path()));
}

#[test]
fn no_available_declared_folder_never_falls_back_to_workspace_carrier() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("missing.code-workspace");
    workspace(&file, &[dir.path().join("missing")]);
    let mut app = AppService::new(dir.path().join("session.json"));
    send(&mut app, Command::OpenWorkspace(file));
    assert!(app.workspace.is_some());
    assert!(app.ai_workspace().is_none());
    assert!(app.ai.workspace().is_none());
    let error = app
        .dispatch(Command::Ai(ai::Command::Start {
            config: ai::Config::default(),
            action: ai::Action::Send("não iniciar processo".into()),
        }))
        .unwrap_err();
    assert!(error.contains("Abra um projeto"), "{error}");
    assert!(!app.ai.any_running());
}

#[test]
fn ai_history_and_drafts_restore_only_in_their_folder_across_window_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let shipping = dir.path().join("shipping");
    let game = dir.path().join("game");
    fs::create_dir(&shipping).unwrap();
    fs::create_dir(&game).unwrap();
    let profile = dir.path().join("profile/session.json");
    let mut first = AppService::new_with_profile(dir.path().join("first.json"), profile.clone());
    send(&mut first, Command::OpenWorkspace(shipping.clone()));
    send(&mut first, Command::Ai(ai::Command::NewConversation));
    let shipping_chat = first.ai.active_chat_id().unwrap().to_owned();
    first
        .ai
        .set_draft("Shipping conversation draft 🦀")
        .unwrap();
    wait(&mut first, |app| !app.ai.profile_pending());
    assert_eq!(first.ai.conversations().len(), 1);
    drop(first);

    let mut second = AppService::new_with_profile(dir.path().join("second.json"), profile.clone());
    wait(&mut second, |app| app.ai.profile_ready());
    assert!(second.ai.workspace().is_none());
    assert!(second.ai.conversations().is_empty());
    assert!(second.ai.entries.is_empty());
    assert!(second.ai.saved_draft().is_none());
    send(&mut second, Command::OpenWorkspace(game.clone()));
    assert_eq!(second.ai.workspace(), Some(game.as_path()));
    assert!(second.ai.conversations().is_empty());
    assert!(second.ai.saved_draft().is_none());
    assert!(second.ai.entries.is_empty());
    assert!(
        second
            .ai
            .dispatch(ai::Command::SelectChat(shipping_chat.clone()), None)
            .is_err()
    );
    send(&mut second, Command::Ai(ai::Command::NewConversation));
    let game_chat = second.ai.active_chat_id().unwrap().to_owned();
    second.ai.set_draft("Game conversation draft").unwrap();
    send(&mut second, Command::OpenWorkspace(shipping.clone()));
    assert_eq!(second.ai.active_chat_id(), Some(shipping_chat.as_str()));
    assert_eq!(
        second.ai.saved_draft(),
        Some("Shipping conversation draft 🦀")
    );
    assert_eq!(second.ai.conversations().len(), 1);
    send(&mut second, Command::CloseWorkspace);
    assert!(second.ai.workspace().is_none());
    assert!(second.ai.conversations().is_empty());
    assert!(second.ai.entries.is_empty());
    assert!(second.ai.saved_draft().is_none());
    wait(&mut second, |app| !app.ai.profile_pending());
    drop(second);

    let mut third = AppService::new_with_profile(dir.path().join("third.json"), profile);
    send(&mut third, Command::OpenWorkspace(game));
    assert_eq!(third.ai.active_chat_id(), Some(game_chat.as_str()));
    assert_eq!(third.ai.saved_draft(), Some("Game conversation draft"));
    assert_eq!(third.ai.conversations().len(), 1);
    assert!(!third.ai.any_running());
}

#[cfg(unix)]
#[test]
fn running_process_and_resumed_thread_stay_in_their_declared_folder_after_tab_switch() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let carrier = dir.path().join("carrier");
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    for root in [&carrier, &first, &second] {
        fs::create_dir(root).unwrap();
    }
    fs::write(first.join("note.txt"), "first").unwrap();
    fs::write(second.join("note.txt"), "second").unwrap();
    let file = carrier.join("team.code-workspace");
    workspace(&file, &[first.clone(), second.clone()]);
    let program = dir.path().join("controlled-codex");
    fs::write(&program, r#"#!/usr/bin/python3
import json,os,sys,time
from pathlib import Path
root=Path.cwd()
def send(v):print(json.dumps(v),flush=True)
for line in sys.stdin:
 v=json.loads(line);m=v.get('method')
 if m=='initialize':send({'id':v['id'],'result':{}})
 elif m in ['thread/start','thread/resume']:
  (root/'binding.json').write_text(json.dumps(v))
  send({'id':v['id'],'result':{'thread':{'id':v['params'].get('threadId','thread-'+root.name)}}})
 elif m=='turn/start':
  (root/'turn.json').write_text(json.dumps(v))
  (root/'waiting').touch()
  send({'id':v['id'],'result':{'turn':{'id':'turn','status':'inProgress'}}})
  while not (root/'release').exists():time.sleep(.003)
  (Path.cwd()/'completed').write_text(os.getcwd())
  send({'method':'item/completed','params':{'item':{'id':'reply','type':'agentMessage','text':root.name}}})
  send({'method':'turn/completed','params':{'turn':{'id':'turn','status':'completed'}}})
 elif m=='account/rateLimits/read':send({'id':v['id'],'result':{'rateLimits':{}}})
"#).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    send(&mut app, Command::OpenWorkspace(file));
    send(&mut app, Command::OpenFile(first.join("note.txt")));
    let config = ai::Config {
        program: program.to_str().unwrap().into(),
        ..Default::default()
    };
    app.dispatch(Command::Ai(ai::Command::Start {
        config: config.clone(),
        action: ai::Action::Send("primeira mensagem".into()),
    }))
    .unwrap();
    wait(&mut app, |_| first.join("waiting").is_file());
    let first_chat = app.ai.active_chat_id().unwrap().to_owned();
    send(&mut app, Command::OpenFile(second.join("note.txt")));
    assert_eq!(app.ai.workspace(), Some(second.as_path()));
    assert_eq!(app.ai.running_count(), 1);
    assert!(!app.ai.is_running());
    assert!(!carrier.join("waiting").exists());
    fs::write(first.join("release"), "").unwrap();
    wait(&mut app, |app| !app.ai.any_running());
    assert_eq!(
        fs::read_to_string(first.join("completed")).unwrap(),
        first.to_str().unwrap()
    );
    assert!(!second.join("completed").exists());
    send(&mut app, Command::OpenFile(first.join("note.txt")));
    assert_eq!(app.ai.active_chat_id(), Some(first_chat.as_str()));
    app.dispatch(Command::Ai(ai::Command::Start {
        config,
        action: ai::Action::Send("segunda mensagem".into()),
    }))
    .unwrap();
    wait(&mut app, |app| !app.ai.any_running());
    let binding: serde_json::Value =
        serde_json::from_slice(&fs::read(first.join("binding.json")).unwrap()).unwrap();
    let turn: serde_json::Value =
        serde_json::from_slice(&fs::read(first.join("turn.json")).unwrap()).unwrap();
    assert_eq!(binding["method"], "thread/resume");
    assert_eq!(binding["params"]["threadId"], "thread-first");
    assert_eq!(binding["params"]["cwd"], first.to_str().unwrap());
    assert_eq!(turn["params"]["cwd"], first.to_str().unwrap());
    assert!(!carrier.join("binding.json").exists());
}

#![cfg(unix)]
use codisseu_ai::{Action, ChatService, Command, Config, MAX_RUNNING_CHATS, Mode, State};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const SCRIPT: &str = r##"#!/usr/bin/python3
import sys,json,os,time,subprocess
from pathlib import Path
root=Path.cwd()
model='unset'
thread=''
child=None
def send(v):print(json.dumps(v),flush=True)
def finish():
 send({'method':'item/completed','params':{'item':{'id':'same-item','type':'agentMessage','text':model+' concluída 🌧'}}})
 n=111 if model=='A' else 222
 b={'totalTokens':n,'inputTokens':n-11,'cachedInputTokens':0,'outputTokens':11,'reasoningOutputTokens':0}
 send({'method':'thread/tokenUsage/updated','params':{'threadId':thread,'tokenUsage':{'total':b,'last':b,'modelContextWindow':1000}}})
 send({'method':'turn/completed','params':{'turn':{'id':'same-turn','status':'completed'}}})
for line in sys.stdin:
 v=json.loads(line);method=v.get('method')
 if method=='initialize':send({'id':v['id'],'result':{}})
 elif method in ['thread/start','thread/resume']:
  model=v['params']['model'];thread=v['params'].get('threadId','thread-'+model)
  (root/(model+'.binding')).write_text(json.dumps(v))
  send({'id':v['id'],'result':{'thread':{'id':thread}}})
 elif method=='turn/start':
  (root/(model+'.turn')).write_text(json.dumps(v))
  (root/(model+'.pid')).write_text(str(os.getpid()))
  send({'id':v['id'],'result':{'turn':{'id':'same-turn','status':'inProgress'}}})
  send({'method':'item/agentMessage/delta','params':{'itemId':'same-item','delta':model+' parcial'}})
  text=v['params']['input'][0]['text']
  if text=='approve':
   send({'id':'same-approval','method':'item/commandExecution/requestApproval','params':{'itemId':'same-tool','command':'echo '+model}})
   continue
  if text=='block':
   child=subprocess.Popen(['/bin/sleep','1000']);(root/(model+'.child')).write_text(str(child.pid))
   (root/(model+'.waiting')).touch()
   while not (root/(model+'.release')).exists():time.sleep(.005)
  finish()
 elif method=='account/rateLimits/read':send({'id':v['id'],'result':{'rateLimits':{'limitId':'codex','primary':{'usedPercent':25}}}})
 elif method=='thread/delete':
  thread=v['params']['threadId'];(root/('deleting-'+thread)).touch()
  while (root/'pause-delete').exists() and not (root/'release-delete').exists():time.sleep(.005)
  (root/('deleted-'+thread)).touch();send({'id':v['id'],'result':{}})
 elif method is None and v.get('id')=='same-approval':
  (root/(model+'.decision')).write_text(json.dumps(v));finish()
"##;
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let program = directory.path().join("cli");
    fs::write(&program, SCRIPT).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    (directory, program)
}
fn config(program: &Path, model: &str, effort: &str) -> Config {
    Config {
        program: program.to_str().unwrap().into(),
        model: model.into(),
        effort: effort.into(),
        mode: Mode::Agent,
        ..Config::default()
    }
}
fn wait_until(chat: &mut ChatService, condition: impl Fn(&ChatService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        chat.poll();
        if condition(chat) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "state={:?} error={:?} profile={:?} active={:?} running={}",
            chat.state,
            chat.error,
            chat.profile_error,
            chat.active_chat_id(),
            chat.running_count()
        );
        std::thread::sleep(Duration::from_millis(3));
    }
}
fn start(chat: &mut ChatService, root: &Path, config: Config, prompt: &str) -> String {
    chat.dispatch(
        Command::Start {
            config,
            action: Action::Send(prompt.into()),
        },
        Some(root.into()),
    )
    .unwrap();
    chat.active_chat_id()
        .expect("identity must exist before process events")
        .into()
}
fn new(chat: &mut ChatService) -> String {
    chat.dispatch(Command::NewConversation, None).unwrap();
    chat.active_chat_id().unwrap().into()
}
fn select(chat: &mut ChatService, id: &str) {
    chat.dispatch(Command::SelectChat(id.into()), None).unwrap();
}
fn no_process(root: &Path, label: &str) {
    for suffix in ["pid", "child"] {
        let path = root.join(format!("{label}.{suffix}"));
        if let Ok(pid) = fs::read_to_string(path) {
            assert!(
                !Path::new(&format!("/proc/{pid}")).exists(),
                "{label} {suffix} remained alive"
            );
        }
    }
}
#[test]
fn two_processes_progress_independently_and_restart_retains_selected_identity() {
    let (workspace, program) = fixture();
    let profile = tempfile::tempdir().unwrap();
    let path = profile.path().join("history.json");
    let mut chat = ChatService::new(path.clone());
    wait_until(&mut chat, |c| c.profile_ready());
    let a_config = config(&program, "A", "high");
    let a = start(&mut chat, workspace.path(), a_config.clone(), "block");
    wait_until(&mut chat, |_| workspace.path().join("A.waiting").exists());
    chat.set_draft("A: próximo pedido 🌧").unwrap();
    let revision = chat.config_revision;
    let b = new(&mut chat);
    assert_ne!(a, b);
    assert_eq!(chat.config, a_config, "new chat inherits selected settings");
    assert!(chat.config_revision > revision);
    assert!(!chat.is_running());
    assert!(chat.entries.is_empty());
    assert_eq!(chat.chat_status(&a), Some(State::Running));
    chat.set_draft("B: próximo pedido ç").unwrap();
    start(
        &mut chat,
        workspace.path(),
        config(&program, "B", "low"),
        "complete",
    );
    assert_eq!(chat.running_count(), 2);
    wait_until(&mut chat, |c| !c.is_running());
    assert_eq!(chat.running_count(), 1);
    assert_eq!(chat.entries.last().unwrap().text, "B concluída 🌧");
    assert_eq!(chat.usage.as_ref().unwrap().total.total_tokens, 222);
    select(&mut chat, &a);
    assert_eq!(chat.config, a_config);
    assert_eq!(chat.session_id(), Some("thread-A"));
    assert_eq!(chat.entries.last().unwrap().text, "A parcial");
    assert!(chat.usage.is_none());
    select(&mut chat, &b);
    fs::write(workspace.path().join("A.release"), "").unwrap();
    wait_until(&mut chat, |c| !c.any_running());
    assert_eq!(chat.active_chat_id(), Some(b.as_str()));
    assert_eq!(chat.entries.last().unwrap().text, "B concluída 🌧");
    assert_eq!(chat.chat_status(&a), Some(State::Completed));
    wait_until(&mut chat, |c| !c.profile_pending());
    drop(chat);
    no_process(workspace.path(), "A");
    no_process(workspace.path(), "B");
    let mut chat = ChatService::new(path);
    chat.sync_workspace(Some(workspace.path()));
    wait_until(&mut chat, |c| c.profile_ready());
    assert_eq!(
        chat.active_chat_id(),
        Some(b.as_str()),
        "background completion changed persisted selection"
    );
    assert!(
        !chat.any_running(),
        "restart cannot claim old workers are still running"
    );
    assert_eq!(chat.saved_draft(), Some("B: próximo pedido ç"));
    select(&mut chat, &a);
    assert_eq!(chat.saved_draft(), Some("A: próximo pedido 🌧"));
    assert_eq!(chat.usage.as_ref().unwrap().total.total_tokens, 111);
    assert_eq!(chat.config, a_config);
    assert_eq!(chat.entries.last().unwrap().text, "A concluída 🌧");
    let cfg = chat.config.clone();
    start(&mut chat, workspace.path(), cfg, "complete");
    wait_until(&mut chat, |c| !c.any_running());
    let binding: Value =
        serde_json::from_slice(&fs::read(workspace.path().join("A.binding")).unwrap()).unwrap();
    assert_eq!(binding["method"], "thread/resume");
    assert_eq!(binding["params"]["threadId"], "thread-A");
}
#[test]
fn identical_approval_ids_cancel_and_configuration_are_scoped_to_selected_chat() {
    let (workspace, program) = fixture();
    let mut chat = ChatService::default();
    let a = start(
        &mut chat,
        workspace.path(),
        config(&program, "A", "high"),
        "approve",
    );
    wait_until(&mut chat, |c| !c.approvals.is_empty());
    new(&mut chat);
    let b = start(
        &mut chat,
        workspace.path(),
        config(&program, "B", "low"),
        "approve",
    );
    wait_until(&mut chat, |c| !c.approvals.is_empty());
    assert_eq!(chat.chat_approval_count(&a), 1);
    assert_eq!(chat.chat_approval_count(&b), 1);
    assert!(
        chat.dispatch(Command::Configure(config(&program, "changed", "low")), None)
            .is_err()
    );
    assert_eq!(chat.config.model, "B");
    select(&mut chat, &a);
    chat.cancel();
    wait_until(&mut chat, |c| !c.is_running());
    assert_eq!(chat.state, State::Cancelled);
    assert_eq!(chat.running_count(), 1);
    assert_eq!(chat.chat_status(&b), Some(State::Running));
    assert!(!workspace.path().join("B.decision").exists());
    select(&mut chat, &b);
    assert_eq!(chat.config.model, "B");
    assert_eq!(chat.approvals.len(), 1);
    chat.dispatch(
        Command::Approve {
            id: json!("same-approval"),
            accept: true,
        },
        None,
    )
    .unwrap();
    wait_until(&mut chat, |c| !c.any_running());
    assert_eq!(chat.state, State::Completed);
    assert!(!workspace.path().join("A.decision").exists());
    assert!(workspace.path().join("B.decision").exists());
    no_process(workspace.path(), "A");
    no_process(workspace.path(), "B");
}
#[test]
fn workspace_switch_keeps_original_cwd_and_direct_drop_reaps_all_owned_processes() {
    let (first, program) = fixture();
    let second = tempfile::tempdir().unwrap();
    let mut chat = ChatService::default();
    let a = start(
        &mut chat,
        first.path(),
        config(&program, "A", "high"),
        "block",
    );
    wait_until(&mut chat, |_| first.path().join("A.waiting").exists());
    chat.sync_workspace(Some(second.path()));
    assert!(!chat.is_running());
    assert!(chat.any_running());
    assert!(chat.entries.is_empty());
    assert!(chat.dispatch(Command::SelectChat(a.clone()), None).is_err());
    start(
        &mut chat,
        second.path(),
        config(&program, "B", "low"),
        "block",
    );
    wait_until(&mut chat, |_| second.path().join("B.waiting").exists());
    assert_eq!(chat.running_count(), 2);
    for (root, model) in [(first.path(), "A"), (second.path(), "B")] {
        let turn: Value =
            serde_json::from_slice(&fs::read(root.join(format!("{model}.turn"))).unwrap()).unwrap();
        assert_eq!(turn["params"]["cwd"], root.to_str().unwrap());
    }
    chat.sync_workspace(Some(first.path()));
    assert_eq!(chat.active_chat_id(), Some(a.as_str()));
    assert_eq!(chat.session_id(), Some("thread-A"));
    drop(chat);
    no_process(first.path(), "A");
    no_process(second.path(), "B");
}
#[test]
fn deleting_idle_chat_runs_in_background_and_delayed_ack_never_resurrects_it() {
    let (workspace, program) = fixture();
    let profile = tempfile::tempdir().unwrap();
    let path = profile.path().join("history.json");
    let mut chat = ChatService::new(path.clone());
    wait_until(&mut chat, |c| c.profile_ready());
    let a = start(
        &mut chat,
        workspace.path(),
        config(&program, "A", "high"),
        "complete",
    );
    wait_until(&mut chat, |c| !c.any_running());
    new(&mut chat);
    let b = start(
        &mut chat,
        workspace.path(),
        config(&program, "B", "low"),
        "block",
    );
    wait_until(&mut chat, |_| workspace.path().join("B.waiting").exists());
    assert!(chat.dispatch(Command::DeleteChat(b.clone()), None).is_err());
    fs::write(workspace.path().join("pause-delete"), "").unwrap();
    chat.dispatch(Command::DeleteChat(a.clone()), None).unwrap();
    assert_eq!(chat.active_chat_id(), Some(b.as_str()));
    assert_eq!(chat.running_count(), 2);
    wait_until(&mut chat, |_| {
        workspace.path().join("deleting-thread-A").exists()
    });
    let deadline = Instant::now() + Duration::from_millis(850);
    while Instant::now() < deadline {
        chat.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    fs::write(workspace.path().join("release-delete"), "").unwrap();
    wait_until(&mut chat, |c| c.chat_status(&a).is_none());
    assert_eq!(chat.active_chat_id(), Some(b.as_str()));
    assert!(chat.is_running());
    chat.cancel_all();
    wait_until(&mut chat, |c| !c.any_running() && !c.profile_pending());
    drop(chat);
    let mut chat = ChatService::new(path);
    chat.sync_workspace(Some(workspace.path()));
    wait_until(&mut chat, |c| c.profile_ready());
    assert!(chat.chat_status(&a).is_none());
    assert_eq!(chat.active_chat_id(), Some(b.as_str()));
}
#[test]
fn active_and_history_limits_refuse_new_work_without_losing_running_or_old_chats() {
    let (workspace, program) = fixture();
    let mut chat = ChatService::default();
    let mut ids = Vec::new();
    for n in 0..MAX_RUNNING_CHATS {
        if n != 0 {
            new(&mut chat);
        }
        ids.push(start(
            &mut chat,
            workspace.path(),
            config(&program, &format!("M{n}"), "high"),
            "block",
        ));
    }
    wait_until(&mut chat, |_| {
        (0..MAX_RUNNING_CHATS).all(|n| workspace.path().join(format!("M{n}.waiting")).exists())
    });
    new(&mut chat);
    assert!(
        chat.dispatch(
            Command::Start {
                config: config(&program, "extra", "high"),
                action: Action::Send("block".into())
            },
            Some(workspace.path().into())
        )
        .is_err()
    );
    assert!(chat.entries.is_empty());
    while chat.conversations().len() < 16 {
        new(&mut chat);
    }
    let before = chat.conversations();
    assert!(
        chat.dispatch(Command::NewConversation, None)
            .unwrap_err()
            .contains("16")
    );
    assert_eq!(chat.conversations(), before);
    for id in ids {
        assert_eq!(chat.chat_status(&id), Some(State::Running));
    }
    assert_eq!(chat.running_count(), MAX_RUNNING_CHATS);
    chat.cancel_all();
    wait_until(&mut chat, |c| !c.any_running());
    assert_eq!(chat.conversations().len(), 16);
    for n in 0..MAX_RUNNING_CHATS {
        no_process(workspace.path(), &format!("M{n}"));
    }
}

#[test]
fn unsent_text_drafts_restart_clear_and_remain_separate_by_chat_and_project() {
    let workspace = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let profile = tempfile::tempdir().unwrap();
    let path = profile.path().join("history.json");
    let mut chat = ChatService::new(path.clone());
    chat.sync_workspace(Some(workspace.path()));
    wait_until(&mut chat, |c| c.profile_ready());
    chat.set_draft("A: ação 🌧\nsem enviar").unwrap();
    let a = chat.active_chat_id().unwrap().to_owned();
    let b = new(&mut chat);
    chat.set_draft("B: rascunho isolado").unwrap();
    chat.sync_workspace(Some(second.path()));
    assert_eq!(chat.saved_draft(), None);
    chat.set_draft("C: outro projeto").unwrap();
    wait_until(&mut chat, |c| !c.profile_pending());
    assert!(!chat.any_running());
    drop(chat);
    let mut chat = ChatService::new(path.clone());
    chat.sync_workspace(Some(workspace.path()));
    wait_until(&mut chat, |c| c.profile_ready());
    assert_eq!(chat.active_chat_id(), Some(b.as_str()));
    assert_eq!(chat.saved_draft(), Some("B: rascunho isolado"));
    select(&mut chat, &a);
    assert_eq!(chat.saved_draft(), Some("A: ação 🌧\nsem enviar"));
    chat.set_draft("").unwrap();
    chat.sync_workspace(Some(second.path()));
    assert_eq!(chat.saved_draft(), Some("C: outro projeto"));
    wait_until(&mut chat, |c| !c.profile_pending());
    drop(chat);
    let mut chat = ChatService::new(path);
    chat.sync_workspace(Some(workspace.path()));
    wait_until(&mut chat, |c| c.profile_ready());
    assert_eq!(chat.active_chat_id(), Some(a.as_str()));
    assert_eq!(chat.saved_draft(), Some(""));
    assert!(chat.entries.is_empty());
    assert!(!chat.any_running());
}

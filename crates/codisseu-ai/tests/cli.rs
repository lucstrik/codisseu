#![cfg(unix)]
use codisseu_ai::{Action, ChatService, Command, Config, Mode, Provider, State};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    time::{Duration, Instant},
};
const SCRIPT: &str = r##"#!/usr/bin/python3
import sys,json,os,time,subprocess
from pathlib import Path
root=Path(__file__).parent
(root/'pid').write_text(str(os.getpid()))
(root/'cwd').write_text(os.getcwd())
(root/'args').write_text(json.dumps(sys.argv[1:]))
mode=(root/'mode').read_text()
def send(v):print(json.dumps(v),flush=True)
if '--output-format' in sys.argv:
 (root/'prompt').write_text(sys.stdin.read())
 send({'type':'system','session_id':'claude-session'})
 send({'type':'stream_event','event':{'type':'message_start','message':{'id':'claude-answer'}}})
 send({'type':'stream_event','event':{'delta':{'type':'text_delta','text':'Olá 🌧'}}})
 send({'type':'assistant','message':{'id':'claude-answer','content':[{'type':'text','text':'Olá 🌧 final'}]}})
 send({'type':'result','result':'Olá 🌧 final','session_id':'claude-session','is_error':mode=='claude-error','permission_denials':[{'tool_name':'Bash'}]})
 sys.exit(7 if mode=='claude-exit' else 0)
for line in sys.stdin:
 v=json.loads(line)
 with (root/'requests').open('a')as f:f.write(json.dumps(v)+'\n')
 method=v.get('method')
 if method=='initialize':send({'id':v['id'],'result':{'userAgent':'fixture'}})
 elif method=='account/read':send({'id':v['id'],'result':{'account':{'type':'chatgpt'}}})
 elif method=='account/rateLimits/read':
  if mode=='usage-error':send({'id':v['id'],'error':{'code':-32601,'message':'not supported'}})
  else:send({'id':v['id'],'result':{'rateLimits':{'limitId':'codex','primary':{'usedPercent':28,'windowDurationMins':300,'resetsAt':int(time.time())+3600},'secondary':{'usedPercent':64,'windowDurationMins':10080,'resetsAt':int(time.time())+7200}}}})
 elif method=='thread/delete':
  if mode=='delete-error':send({'id':v['id'],'error':{'code':-32000,'message':'fixture delete rejected'}})
  else:(root/'deleted').write_text(v['params']['threadId']);send({'id':v['id'],'result':{}})
 elif method=='model/list':
  cursor=v['params'].get('cursor')
  if cursor is None:send({'id':v['id'],'result':{'data':[{'id':'fixture-vision','displayName':'Fixture Vision','isDefault':True,'defaultReasoningEffort':'high','supportedReasoningEfforts':[{'reasoningEffort':'high'}],'inputModalities':['text','image']}],'nextCursor':'page2'}})
  else:send({'id':v['id'],'result':{'data':[{'id':'fixture-hidden','displayName':'Hidden','hidden':True},{'id':'fixture-text','displayName':'Fixture Text'}],'nextCursor':None}})
 elif method in ['thread/start','thread/resume']:
  send({'id':v['id'],'result':{'thread':{'id':'thread-fixture'}}})
  if mode=='hang-input':(root/'not-reading').touch();time.sleep(1000)
 elif method=='turn/start':
  for item in v['params']['input']:
   if item['type']=='localImage':
    data=Path(item['path']).read_bytes();assert data.startswith(b'\x89PNG');(root/'image-seen').write_text(item['path'])
  sys.stderr.write('PRIVATE-DIAGNOSTIC\n');sys.stderr.flush()
  if mode=='crash':sys.exit(6)
  if mode=='bad-json':print('not json',flush=True);time.sleep(10)
  if mode=='oversize':print('x'*2200000,flush=True);time.sleep(10)
  if mode=='hang':
   child=subprocess.Popen(['/bin/sleep','1000']);(root/'child-pid').write_text(str(child.pid));time.sleep(1000)
  if mode=='approve':
   send({'id':'approval-opaque','method':'item/commandExecution/requestApproval','params':{'itemId':'tool-1','command':'echo fixture','reason':'fixture only'}})
   continue
  if mode=='unknown':
   send({'id':99,'method':'tool/unknown','params':{}});continue
  send({'id':v['id'],'result':{'turn':{'id':'turn-1','status':'inProgress'}}})
  if mode=='foreign-events':
   send({'id':'foreign-approval','method':'item/commandExecution/requestApproval','params':{'threadId':'unrelated-thread','turnId':'turn-1','itemId':'foreign-tool','command':'foreign-command'}})
   send({'method':'item/agentMessage/delta','params':{'threadId':'unrelated-thread','turnId':'turn-1','itemId':'answer','delta':'FOREIGN-THREAD'}})
   send({'method':'item/completed','params':{'threadId':'unrelated-thread','turnId':'turn-1','item':{'id':'answer','type':'agentMessage','text':'FOREIGN-THREAD-FINAL'}}})
   send({'method':'turn/completed','params':{'threadId':'unrelated-thread','turn':{'id':'turn-1','status':'completed'}}})
   send({'method':'item/agentMessage/delta','params':{'threadId':'thread-fixture','turnId':'old-turn','itemId':'answer','delta':'OLD-TURN'}})
   send({'method':'item/completed','params':{'threadId':'thread-fixture','turnId':'old-turn','item':{'id':'answer','type':'agentMessage','text':'OLD-TURN-FINAL'}}})
   send({'method':'turn/completed','params':{'threadId':'thread-fixture','turn':{'id':'old-turn','status':'failed','error':{'message':'OLD-TURN-FAILED'}}}})
  send({'method':'item/agentMessage/delta','params':{'itemId':'answer','delta':'Olá 🌧'}})
  send({'method':'item/completed','params':{'item':{'id':'answer','type':'agentMessage','text':'Olá 🌧 final'}}})
  if mode=='quota-stream':
   send({'method':'account/rateLimits/updated','params':{'rateLimits':{'limitId':'codex','primary':{'usedPercent':40}}}})
   time.sleep(.2)
  breakdown={'totalTokens':8421,'inputTokens':8000,'cachedInputTokens':4000,'outputTokens':421,'reasoningOutputTokens':100}
  send({'method':'thread/tokenUsage/updated','params':{'threadId':'thread-fixture','turnId':'turn-1','tokenUsage':{'total':breakdown,'last':breakdown,'modelContextWindow':100000}}})
  send({'method':'thread/tokenUsage/updated','params':{'threadId':'unrelated-thread','turnId':'turn-1','tokenUsage':{'total':dict(breakdown,totalTokens=999999),'last':breakdown,'modelContextWindow':100000}}})
  send({'method':'turn/completed','params':{'turn':{'id':'turn-1','status':'completed'}}})
 elif v.get('id')=='foreign-approval' and method is None:
  (root/'foreign-decision').write_text(json.dumps(v))
 elif v.get('id') in ['approval-opaque',99] and method is None:
  (root/'decision').write_text(json.dumps(v))
  send({'method':'turn/completed','params':{'turn':{'id':'turn-1','status':'completed'}}})
"##;
fn setup(mode: &str) -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("fake-cli");
    fs::write(&p, SCRIPT).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(dir.path().join("mode"), mode).unwrap();
    (
        dir,
        Config {
            program: p.display().to_string(),
            mode: Mode::Consult,
            ..Config::default()
        },
    )
}
fn wait_until(chat: &mut ChatService, condition: impl Fn(&ChatService) -> bool) {
    let start = Instant::now();
    loop {
        chat.poll();
        if condition(chat) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{:?} {:?}",
            chat.state,
            chat.error
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn wait(chat: &mut ChatService) {
    wait_until(chat, |c| !c.is_running())
}
fn send(chat: &mut ChatService, cfg: Config, root: &Path) {
    chat.dispatch(
        Command::Start {
            config: cfg,
            action: Action::Send("ação 🌧 $(touch NEVER) `literal`".into()),
        },
        Some(root.into()),
    )
    .unwrap();
}
#[test]
fn jsonl_resume_policy_cwd_literal_and_no_duplicate() {
    let (d, cfg) = setup("normal");
    let mut c = ChatService::default();
    send(&mut c, cfg.clone(), d.path());
    wait(&mut c);
    assert_eq!(c.state, State::Completed);
    assert_eq!(
        c.entries.iter().filter(|e| e.role == "Assistente").count(),
        1
    );
    assert_eq!(c.entries.last().unwrap().text, "Olá 🌧 final");
    assert_eq!(
        fs::read_to_string(d.path().join("cwd")).unwrap(),
        d.path().to_str().unwrap()
    );
    assert!(!d.path().join("NEVER").exists());
    send(&mut c, cfg, d.path());
    wait(&mut c);
    let req: Vec<Value> = fs::read_to_string(d.path().join("requests"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(
        req.iter().filter(|v| v["method"] == "initialized").count(),
        2
    );
    let resume = req.iter().find(|v| v["method"] == "thread/resume").unwrap();
    assert_eq!(resume["params"]["threadId"], "thread-fixture");
    assert_eq!(resume["params"]["sandbox"], "read-only");
    let turn = req.iter().find(|v| v["method"] == "turn/start").unwrap();
    assert_eq!(turn["params"]["approvalPolicy"], "never");
    assert_eq!(turn["params"]["sandboxPolicy"]["type"], "readOnly");
    assert!(
        turn["params"]["input"][0]["text"]
            .as_str()
            .unwrap()
            .contains("$(touch NEVER)")
    );
    assert!(
        c.entries
            .iter()
            .all(|e| !e.text.contains("PRIVATE-DIAGNOSTIC"))
    );
}
#[test]
fn relative_workspace_is_resolved_before_process_context_and_protocol() {
    let current = std::env::current_dir().unwrap();
    let workspace = tempfile::Builder::new()
        .prefix("codisseu-ai-relative-")
        .tempdir_in(&current)
        .unwrap();
    let canonical = workspace.path().canonicalize().unwrap();
    let relative = canonical.strip_prefix(&current).unwrap();
    fs::write(workspace.path().join("context.txt"), "contexto relativo 🌧").unwrap();
    let (cli, config) = setup("normal");
    let mut chat = ChatService::default();
    chat.dispatch(
        Command::Start {
            config,
            action: Action::SendWithAttachments {
                text: "Leia o contexto".into(),
                display_text: "Leia o contexto".into(),
                attachments: vec![codisseu_ai::attachments::AttachmentInput::ProjectFile(
                    "context.txt".into(),
                )],
            },
        },
        Some(relative.into()),
    )
    .unwrap();
    wait(&mut chat);
    assert_eq!(chat.state, State::Completed, "{:?}", chat.error);
    assert_eq!(
        fs::read_to_string(cli.path().join("cwd")).unwrap(),
        canonical.to_str().unwrap()
    );
    let requests: Vec<Value> = fs::read_to_string(cli.path().join("requests"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for method in ["thread/start", "turn/start"] {
        let request = requests
            .iter()
            .find(|request| request["method"] == method)
            .unwrap();
        assert_eq!(request["params"]["cwd"], json!(canonical), "{method}");
    }
    let turn = requests
        .iter()
        .find(|request| request["method"] == "turn/start")
        .unwrap();
    assert!(
        turn["params"]["input"][0]["text"]
            .as_str()
            .unwrap()
            .contains("contexto relativo 🌧")
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join("context.txt")).unwrap(),
        "contexto relativo 🌧"
    );
}
#[test]
fn foreign_thread_and_old_turn_events_cannot_complete_or_pollute_selected_chat() {
    let (directory, mut config) = setup("foreign-events");
    config.mode = Mode::Agent;
    let mut chat = ChatService::default();
    send(&mut chat, config, directory.path());
    wait(&mut chat);
    assert_eq!(chat.state, State::Completed, "{:?}", chat.error);
    assert!(chat.approvals.is_empty());
    assert_eq!(chat.entries.last().unwrap().text, "Olá 🌧 final");
    assert!(
        chat.entries
            .iter()
            .all(|entry| !entry.text.contains("FOREIGN") && !entry.text.contains("OLD-TURN"))
    );
    let decision: Value = serde_json::from_str(
        &fs::read_to_string(directory.path().join("foreign-decision")).unwrap(),
    )
    .unwrap();
    assert!(
        decision.get("result").is_none(),
        "foreign request cannot be approved"
    );
    assert_eq!(decision["error"]["code"], -32602);
    assert!(
        chat.notice
            .as_ref()
            .is_none_or(|notice| !notice.contains("foreign"))
    );
}
#[test]
fn approvals_require_explicit_matching_response() {
    for accept in [false, true] {
        let (d, mut cfg) = setup("approve");
        cfg.mode = Mode::Agent;
        let mut c = ChatService::default();
        send(&mut c, cfg, d.path());
        wait_until(&mut c, |c| !c.approvals.is_empty());
        assert!(!d.path().join("decision").exists());
        assert!(
            c.dispatch(
                Command::Approve {
                    id: json!("stale"),
                    accept
                },
                None
            )
            .is_err()
        );
        c.dispatch(
            Command::Approve {
                id: json!("approval-opaque"),
                accept,
            },
            None,
        )
        .unwrap();
        wait(&mut c);
        let result: Value =
            serde_json::from_str(&fs::read_to_string(d.path().join("decision")).unwrap()).unwrap();
        assert_eq!(
            result["result"]["decision"],
            if accept { "accept" } else { "decline" }
        );
        assert_eq!(c.state, State::Completed);
    }
}
#[test]
fn unknown_requests_fail_closed() {
    let (d, cfg) = setup("unknown");
    let mut c = ChatService::default();
    send(&mut c, cfg, d.path());
    wait(&mut c);
    assert!(c.notice.as_ref().unwrap().contains("não suportada"));
    let decision: Value =
        serde_json::from_str(&fs::read_to_string(d.path().join("decision")).unwrap()).unwrap();
    assert_eq!(decision["error"]["code"], -32601);
}
#[test]
fn malformed_oversize_crash_and_missing_program_are_failures() {
    for mode in ["crash", "bad-json", "oversize"] {
        let (d, cfg) = setup(mode);
        let mut c = ChatService::default();
        send(&mut c, cfg, d.path());
        wait(&mut c);
        assert_eq!(c.state, State::Failed, "{mode}");
        assert!(c.error.is_some());
    }
    let d = tempfile::tempdir().unwrap();
    let mut c = ChatService::default();
    send(
        &mut c,
        Config {
            program: "/no/such/cli".into(),
            ..Config::default()
        },
        d.path(),
    );
    wait(&mut c);
    assert_eq!(c.state, State::Failed);
}
#[test]
fn cancel_reaps_owned_process_and_workspace_does_not_reuse_session() {
    let (d, cfg) = setup("hang");
    let mut c = ChatService::default();
    send(&mut c, cfg, d.path());
    wait_until(&mut c, |_| d.path().join("child-pid").exists());
    let pid = fs::read_to_string(d.path().join("pid")).unwrap();
    c.cancel();
    assert_eq!(c.state, State::Cancelling);
    wait(&mut c);
    assert_eq!(c.state, State::Cancelled);
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
    c.sync_workspace(None);
    assert!(c.entries.is_empty());
    assert!(c.session_id().is_none());
}
#[test]
fn claude_structured_stream_and_resume_preserve_partial_final_identity() {
    let (d, mut cfg) = setup("normal");
    cfg.provider = Provider::Claude;
    let mut c = ChatService::default();
    send(&mut c, cfg.clone(), d.path());
    wait(&mut c);
    assert_eq!(c.state, State::Completed, "{:?}", c.error);
    assert_eq!(
        c.entries.iter().filter(|e| e.role == "Assistente").count(),
        1
    );
    assert_eq!(c.entries.last().unwrap().text, "Olá 🌧 final");
    assert_eq!(c.session_id(), Some("claude-session"));
    assert!(c.notice.is_some());
    send(&mut c, cfg, d.path());
    wait(&mut c);
    let args: Vec<String> =
        serde_json::from_str(&fs::read_to_string(d.path().join("args")).unwrap()).unwrap();
    assert!(args.windows(2).any(|w| w == ["--resume", "claude-session"]));
    assert!(
        args.windows(2)
            .any(|w| w == ["--permission-mode", "dontAsk"])
    );
    assert!(args.windows(2).any(|w| w == ["--tools", "Read,Glob,Grep"]));
    assert!(
        fs::read_to_string(d.path().join("prompt"))
            .unwrap()
            .contains("`literal`")
    );
}
#[test]
fn claude_error_or_nonzero_exit_is_not_success() {
    for mode in ["claude-error", "claude-exit"] {
        let (d, mut cfg) = setup(mode);
        cfg.provider = Provider::Claude;
        let mut c = ChatService::default();
        send(&mut c, cfg, d.path());
        wait(&mut c);
        assert_eq!(c.state, State::Failed);
    }
}

#[test]
fn cancel_interrupts_a_cli_that_does_not_read_a_large_prompt() {
    let (d, cfg) = setup("hang-input");
    let mut c = ChatService::default();
    c.dispatch(
        Command::Start {
            config: cfg,
            action: Action::Send("x".repeat(codisseu_ai::MAX_PROMPT)),
        },
        Some(d.path().into()),
    )
    .unwrap();
    wait_until(&mut c, |_| d.path().join("not-reading").exists());
    c.cancel();
    wait(&mut c);
    assert_eq!(c.state, State::Cancelled);
    let pid = fs::read_to_string(d.path().join("pid")).unwrap();
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
}

#[test]
fn models_multimodal_and_turn_overrides_use_real_protocol_fields() {
    use codisseu_ai::attachments::AttachmentInput;
    let (d, mut cfg) = setup("normal");
    let mut c = ChatService::default();
    c.dispatch(
        Command::Start {
            config: cfg.clone(),
            action: Action::Models,
        },
        Some(d.path().into()),
    )
    .unwrap();
    wait(&mut c);
    assert_eq!(c.state, State::Completed, "{:?}", c.error);
    assert_eq!(
        c.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["fixture-vision", "fixture-text"]
    );
    assert!(c.models[0].images);
    assert!(c.entries.is_empty());
    cfg.model = "fixture-vision".into();
    cfg.effort = "high".into();
    cfg.mode = Mode::Agent;
    let source = d.path().join("context.txt");
    fs::write(&source, "contexto ação-917").unwrap();
    c.dispatch(
        Command::Start {
            config: cfg.clone(),
            action: Action::SendWithAttachments {
                text: "Qual cor?".into(),
                display_text: "Qual cor?".into(),
                attachments: vec![
                    AttachmentInput::ProjectFile(source),
                    AttachmentInput::ImageRgba {
                        width: 2,
                        height: 2,
                        rgba: [255, 0, 0, 255].repeat(4),
                        name: "clipboard.png".into(),
                    },
                ],
            },
        },
        Some(d.path().into()),
    )
    .unwrap();
    wait(&mut c);
    assert_eq!(c.state, State::Completed, "{:?}", c.error);
    let image = fs::read_to_string(d.path().join("image-seen")).unwrap();
    assert!(
        !Path::new(&image).exists(),
        "temporary image survived process cleanup"
    );
    let requests: Vec<Value> = fs::read_to_string(d.path().join("requests"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r["method"] == "model/list")
            .count(),
        2
    );
    let turn = &requests
        .iter()
        .find(|r| r["method"] == "turn/start")
        .unwrap()["params"];
    assert_eq!(turn["cwd"], d.path().to_str().unwrap());
    assert_eq!(turn["model"], "fixture-vision");
    assert_eq!(turn["effort"], "high");
    assert_eq!(turn["sandboxPolicy"]["type"], "workspaceWrite");
    assert_eq!(turn["approvalPolicy"], "on-request");
    assert!(
        turn["input"][0]["text"]
            .as_str()
            .unwrap()
            .contains("contexto ação-917")
    );
    assert_eq!(turn["input"][1]["type"], "localImage");
    cfg.model = "fixture-text".into();
    cfg.effort = "low".into();
    send(&mut c, cfg, d.path());
    wait(&mut c);
    let requests: Vec<Value> = fs::read_to_string(d.path().join("requests"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(
        requests
            .iter()
            .rev()
            .find(|r| r["method"] == "turn/start")
            .unwrap()["params"]["effort"],
        "low"
    );
    assert!(requests.iter().any(|r| r["method"] == "thread/resume"));
}
#[test]
fn history_reopens_resumes_and_isolates_projects() {
    let (d, cfg) = setup("normal");
    let profile = tempfile::tempdir().unwrap();
    let path = profile.path().join("chat.json");
    let second = tempfile::tempdir().unwrap();
    let mut c = ChatService::new(path.clone());
    wait_until(&mut c, |c| c.profile_ready());
    send(&mut c, cfg, d.path());
    wait(&mut c);
    wait_until(&mut c, |c| !c.profile_pending());
    let transcript = c.entries.iter().map(|e| e.text.clone()).collect::<Vec<_>>();
    c.sync_workspace(Some(second.path()));
    assert!(c.entries.is_empty());
    assert!(c.session_id().is_none());
    c.sync_workspace(Some(d.path()));
    assert_eq!(
        c.entries.iter().map(|e| e.text.clone()).collect::<Vec<_>>(),
        transcript
    );
    wait_until(&mut c, |c| !c.profile_pending());
    drop(c);
    let mut c = ChatService::new(path);
    c.sync_workspace(Some(d.path()));
    wait_until(&mut c, |c| c.profile_ready());
    assert_eq!(
        c.entries.iter().map(|e| e.text.clone()).collect::<Vec<_>>(),
        transcript
    );
    assert_eq!(c.session_id(), Some("thread-fixture"));
    let cfg = c.config.clone();
    send(&mut c, cfg, d.path());
    wait(&mut c);
    assert_eq!(c.state, State::Completed);
    let req = fs::read_to_string(d.path().join("requests")).unwrap();
    assert!(req.contains("thread/resume"));
    let first = c.conversations()[0].0.clone();
    c.dispatch(Command::NewConversation, None).unwrap();
    assert!(c.entries.is_empty());
    c.dispatch(Command::SelectChat(first.clone()), None)
        .unwrap();
    assert!(!c.entries.is_empty());
    c.sync_workspace(Some(second.path()));
    assert!(c.dispatch(Command::SelectChat(first), None).is_err());
    wait_until(&mut c, |c| !c.profile_pending());
}

#[test]
fn full_access_is_explicit_sent_on_resume_and_persisted_with_usage() {
    let (d, mut cfg) = setup("normal");
    let profile = tempfile::tempdir().unwrap();
    let path = profile.path().join("chat.json");
    let mut c = ChatService::new(path.clone());
    wait_until(&mut c, |c| c.profile_ready());
    assert_eq!(Config::default().mode, Mode::Agent);
    cfg.mode = Mode::FullAccess;
    send(&mut c, cfg.clone(), d.path());
    wait(&mut c);
    assert_eq!(c.state, State::Completed);
    assert_eq!(c.usage.as_ref().unwrap().total.total_tokens, 8421);
    assert_eq!(
        c.limits
            .as_ref()
            .unwrap()
            .remaining(codisseu_ai::usage::now()),
        Some(36.)
    );
    wait_until(&mut c, |c| !c.profile_pending());
    drop(c);
    let mut c = ChatService::new(path);
    c.sync_workspace(Some(d.path()));
    wait_until(&mut c, |c| c.profile_ready());
    assert_eq!(c.config.mode, Mode::FullAccess);
    assert_eq!(c.usage.as_ref().unwrap().total.total_tokens, 8421);
    assert!(c.limits.is_none());
    send(&mut c, cfg.clone(), d.path());
    wait(&mut c);
    cfg.mode = Mode::Consult;
    send(&mut c, cfg, d.path());
    wait(&mut c);
    let req: Vec<Value> = fs::read_to_string(d.path().join("requests"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let turns: Vec<_> = req.iter().filter(|r| r["method"] == "turn/start").collect();
    assert_eq!(
        turns[0]["params"]["sandboxPolicy"],
        json!({"type":"dangerFullAccess"})
    );
    assert_eq!(turns[0]["params"]["approvalPolicy"], "never");
    assert_eq!(
        turns[1]["params"]["sandboxPolicy"]["type"],
        "dangerFullAccess"
    );
    assert_eq!(turns[2]["params"]["sandboxPolicy"]["type"], "readOnly");
    assert_eq!(
        req.iter().find(|r| r["method"] == "thread/start").unwrap()["params"]["sandbox"],
        "danger-full-access"
    );
    assert_eq!(
        req.iter().find(|r| r["method"] == "thread/resume").unwrap()["params"]["sandbox"],
        "danger-full-access"
    );
}
#[test]
fn deleting_active_conversation_acknowledges_cli_and_never_resurrects_on_restart() {
    let (d, cfg) = setup("normal");
    let profile = tempfile::tempdir().unwrap();
    let path = profile.path().join("chat.json");
    let mut c = ChatService::new(path.clone());
    wait_until(&mut c, |c| c.profile_ready());
    send(&mut c, cfg.clone(), d.path());
    wait(&mut c);
    let id = c.conversations()[0].0.clone();
    c.dispatch(Command::DeleteChat(id.clone()), None).unwrap();
    wait(&mut c);
    assert!(c.entries.is_empty());
    assert!(c.session_id().is_none());
    assert!(c.usage.is_none());
    assert!(c.conversations().is_empty());
    assert_eq!(
        fs::read_to_string(d.path().join("deleted")).unwrap(),
        "thread-fixture"
    );
    wait_until(&mut c, |c| !c.profile_pending());
    drop(c);
    let mut c = ChatService::new(path);
    c.sync_workspace(Some(d.path()));
    wait_until(&mut c, |c| c.profile_ready());
    assert!(c.entries.is_empty());
    assert!(c.conversations().is_empty());
    send(&mut c, cfg, d.path());
    wait(&mut c);
    assert!(c.conversations().iter().all(|(other, _)| other != &id));
}
#[test]
fn deletion_failure_or_foreign_project_keeps_conversation() {
    let (d, cfg) = setup("delete-error");
    let mut c = ChatService::default();
    send(&mut c, cfg, d.path());
    wait(&mut c);
    let id = c.conversations()[0].0.clone();
    let old = c.entries.len();
    c.dispatch(Command::DeleteChat(id.clone()), None).unwrap();
    wait(&mut c);
    assert_eq!(c.state, State::Failed);
    assert_eq!(c.entries.len(), old);
    assert_eq!(c.conversations().len(), 1);
    assert!(c.error.as_ref().unwrap().contains("delete rejected"));
    let other = tempfile::tempdir().unwrap();
    c.sync_workspace(Some(other.path()));
    assert!(c.dispatch(Command::DeleteChat(id), None).is_err());
}
#[test]
fn optional_limits_failure_does_not_fail_a_successful_turn_and_claude_full_uses_all_tools() {
    let (d, cfg) = setup("usage-error");
    let mut c = ChatService::default();
    send(&mut c, cfg, d.path());
    wait(&mut c);
    assert_eq!(c.state, State::Completed);
    assert!(c.usage.is_some());
    assert!(c.limits.is_none());
    assert!(c.limits_error.as_ref().unwrap().contains("not supported"));
    let (d, mut cfg) = setup("normal");
    cfg.provider = Provider::Claude;
    cfg.mode = Mode::FullAccess;
    let mut c = ChatService::default();
    send(&mut c, cfg, d.path());
    wait(&mut c);
    let args: Vec<String> =
        serde_json::from_str(&fs::read_to_string(d.path().join("args")).unwrap()).unwrap();
    assert!(
        args.windows(2)
            .any(|w| w == ["--permission-mode", "bypassPermissions"])
    );
    assert!(args.windows(2).any(|w| w == ["--tools", "default"]));
    assert!(!args.iter().any(|a| a == "--allowedTools"));
    let id = c.conversations()[0].0.clone();
    c.dispatch(Command::DeleteChat(id), None).unwrap();
    assert!(c.conversations().is_empty());
    assert!(c.notice.as_ref().unwrap().contains("histórico externo"));
}

#[test]
fn account_quota_notification_is_visible_while_a_turn_is_still_running() {
    let (d, cfg) = setup("quota-stream");
    let mut c = ChatService::default();
    send(&mut c, cfg, d.path());
    wait_until(&mut c, |c| {
        c.limits
            .as_ref()
            .and_then(|l| l.remaining(codisseu_ai::usage::now()))
            == Some(60.)
    });
    assert!(c.is_running());
    wait(&mut c);
    assert_eq!(c.state, State::Completed);
    assert_eq!(
        c.limits
            .as_ref()
            .unwrap()
            .remaining(codisseu_ai::usage::now()),
        Some(36.)
    );
}

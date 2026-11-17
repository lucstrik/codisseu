//! Explicit CEF + private vault acceptance and a bounded multi-page workload.
use super::*;
use crate::browser_library::Action;
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
};
struct Http {
    origin: String,
    stop: Arc<AtomicBool>,
    task: Option<thread::JoinHandle<()>>,
}
impl Http {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let task = thread::spawn(move || {
            while !s.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut buf = [0; 4096];
                        let _ = stream.read(&mut buf);
                        let page = r#"<!doctype html><title>Owned login fixture</title><meta charset=utf-8><style>body{font:16px sans-serif;background:#eee}input,button{padding:10px;margin:8px}#heavy{display:grid;grid-template-columns:repeat(10,1fr)}</style><form onsubmit='event.preventDefault();window.submitted=(window.submitted||0)+1'><input id=u autocomplete=username value='fixture@example.invalid'><input id=p type=password autocomplete=current-password value='owned-fixture-only'><button>Login</button></form><div id=heavy></div><script>window.submitted=0;if(location.pathname.includes('heavy')){heavy.innerHTML=Array.from({length:5000},(_,i)=>'<div>Cell '+i+'</div>').join('');setInterval(()=>{heavy.firstChild.textContent=String(Date.now())},100)}</script>"#;
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
                            page.len()
                        );
                        let _ = stream.write_all(response.as_bytes());
                    }
                    Err(_) => thread::sleep(Duration::from_millis(2)),
                }
            }
        });
        Self {
            origin,
            stop,
            task: Some(task),
        }
    }
}
impl Drop for Http {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.origin.trim_start_matches("http://"));
        if let Some(t) = self.task.take() {
            t.join().unwrap();
        }
    }
}
fn wait(a: &mut AppService, p: impl Fn(&AppService) -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        a.poll();
        assert_ne!(
            a.browser.state,
            browser::EngineState::Failed,
            "{:?}",
            a.browser.last_error
        );
        if p(a) {
            return;
        }
        assert!(
            Instant::now() < end,
            "browser timeout {:?} {:?}",
            a.browser_login_notice(),
            a.browser_library.error
        );
        thread::sleep(Duration::from_millis(3));
    }
}
fn eval(a: &mut AppService, id: u64, text: &str) -> Value {
    let epoch = a.reserve_browser_cdp_epoch().unwrap();
    let channel = a.browser.devtools(id, epoch).unwrap();
    let mut p = Page {
        channel,
        serial: 0,
        contexts: BTreeMap::new(),
        stop: Arc::new(AtomicBool::new(false)),
        deadline: Instant::now() + Duration::from_secs(5),
    };
    p.call(
        "Runtime.evaluate",
        json!({"expression":text,"returnByValue":true}),
    )
    .unwrap()["result"]["value"]
        .clone()
}
fn library(a: &mut AppService, action: Action) {
    a.browser_library.dispatch(action).unwrap();
    wait(a, |a| !a.browser_library.is_busy());
    assert!(
        a.browser_library.error.is_none(),
        "{:?}",
        a.browser_library.error
    );
}
fn open(a: &mut AppService, id: u64, url: &str) {
    a.dispatch(Command::Browser(browser::Command::Open {
        id,
        url: url.into(),
    }))
    .unwrap();
    a.dispatch(Command::Browser(browser::Command::Resize {
        id,
        surface: browser::Surface::Page,
        width: 800,
        height: 600,
        scale: 1.,
    }))
    .unwrap();
    a.dispatch(Command::Browser(browser::Command::Visible {
        id,
        surface: browser::Surface::Page,
        visible: true,
    }))
    .unwrap();
    wait(a, |a| {
        a.browser.state == browser::EngineState::Ready
            && a.browser
                .tabs
                .get(&id)
                .is_some_and(|t| !t.loading && t.url == url)
            && a.browser.frames.contains_key(&(id, browser::Surface::Page))
    });
}
fn rss(pid: u32) -> u64 {
    fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|s| s.starts_with("VmRSS:"))
                .and_then(|s| s.split_whitespace().nth(1))
                .and_then(|s| s.parse::<u64>().ok())
        })
        .unwrap_or(0)
        * 1024
}
fn descendants(parent: u32) -> Vec<u32> {
    let mut pids = vec![parent];
    for _ in 0..16 {
        let before = pids.len();
        for entry in fs::read_dir("/proc").unwrap().flatten() {
            if let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>()
                && !pids.contains(&pid)
                && let Ok(status) = fs::read_to_string(entry.path().join("status"))
                && let Some(ppid) = status
                    .lines()
                    .find(|s| s.starts_with("PPid:"))
                    .and_then(|s| s.split_whitespace().nth(1))
                    .and_then(|s| s.parse::<u32>().ok())
                && pids.contains(&ppid)
            {
                pids.push(pid);
            }
        }
        if pids.len() == before {
            break;
        }
    }
    pids
}
#[test]
#[ignore = "real private CEF156, OS vault and private Xvfb; explicit corpus only"]
fn cef_saved_access_origin_isolation_restart_and_bounded_workload() {
    assert_eq!(
        std::env::var("CODISSEU_BROWSER_VAULT_TEST").as_deref(),
        Ok("1")
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let http = Http::new();
    let other = Http::new();
    let mut a = AppService::new(path.clone());
    wait(&mut a, |a| !a.is_busy());
    let url = format!("{}/login", http.origin);
    let start = Instant::now();
    open(&mut a, 1, &url);
    let cold_ms = start.elapsed().as_millis();
    a.read_browser_login(1).unwrap();
    wait(&mut a, |a| !a.browser_login_busy());
    let captured = a
        .take_browser_login()
        .unwrap_or_else(|| panic!("{:?}", a.browser_login_notice()));
    assert!(captured.username == "fixture@example.invalid");
    assert!(captured.password.expose() == "owned-fixture-only");
    library(
        &mut a,
        Action::SaveLogin {
            origin: captured.origin,
            username: captured.username,
            password: captured.password,
        },
    );
    let key = a.browser_library.state.logins[0].id.clone();
    library(
        &mut a,
        Action::SaveBookmark {
            id: None,
            name: "Local frontend".into(),
            url: url.clone(),
            project: "own-project".into(),
        },
    );
    assert_eq!(
        eval(
            &mut a,
            1,
            "document.getElementById('u').value='';document.getElementById('p').value='';true"
        ),
        json!(true)
    );
    a.fill_browser_login(1, &key).unwrap();
    wait(&mut a, |a| !a.browser_login_busy());
    assert!(
        a.browser_login_notice()
            .unwrap()
            .starts_with("Acesso preenchido"),
        "{:?}",
        a.browser_login_notice()
    );
    assert_eq!(
        eval(
            &mut a,
            1,
            "[document.getElementById('u').value==='fixture@example.invalid',document.getElementById('p').value==='owned-fixture-only',window.submitted]"
        ),
        json!([true, true, 0])
    );
    open(&mut a, 2, &format!("{}/login", other.origin));
    assert!(a.fill_browser_login(2, &key).is_err());
    a.read_browser_login(1).unwrap();
    a.dispatch(Command::Browser(browser::Command::Navigate {
        id: 1,
        url: format!("{}/next", other.origin),
    }))
    .unwrap();
    assert!(!a.browser_login_busy());
    assert!(a.take_browser_login().is_none());
    let mut page_ms = vec![];
    let mut polls = vec![];
    let mut max_frames = 0;
    let mut max_bytes = 0;
    for id in 3..=8 {
        let before = Instant::now();
        open(&mut a, id, &format!("{}/heavy", http.origin));
        page_ms.push(before.elapsed().as_millis());
        for other in 1..id {
            a.dispatch(Command::Browser(browser::Command::Visible {
                id: other,
                surface: browser::Surface::Page,
                visible: false,
            }))
            .unwrap();
        }
        for _ in 0..40 {
            let now = Instant::now();
            a.poll();
            polls.push(now.elapsed().as_micros());
            max_frames = max_frames.max(a.browser.frames.len());
            max_bytes = max_bytes.max(
                a.browser
                    .frames
                    .values()
                    .map(|f| f.bgra.len() + f.rgba.len())
                    .sum::<usize>(),
            );
            thread::sleep(Duration::from_millis(4));
        }
    }
    polls.sort_unstable();
    let pid = a.browser.engine_pid().unwrap();
    let broker_rss = rss(pid);
    let children = descendants(pid);
    let summed_rss = children.iter().map(|pid| rss(*pid)).sum::<u64>();
    let engine_processes = children.len();
    let engine_sample = fs::read_to_string("/proc/cpuinfo").ok().and_then(|s| {
        s.lines()
            .find(|s| s.starts_with("model name"))
            .map(str::to_owned)
    });
    assert!(max_bytes <= 64 * 1024 * 1024);
    assert!(a.browser.tabs.len() == 8);
    let metrics = json!({"kind":"real CEF156 OSR; local login + 5000 DOM cells; release/debug profile recorded by runner","cold_start_first_frame_ms":cold_ms,"heavy_new_tab_ms":page_ms,"samples":polls.len(),"app_poll_us":{"p50":polls[polls.len()/2],"p95":polls[polls.len()*95/100],"max":polls.last()},"peak_public_frames":max_frames,"peak_public_frame_bytes":max_bytes,"broker_rss_bytes":broker_rss,"engine_processes":engine_processes,"sum_rss_bytes_including_shared_pages":summed_rss,"cpu":engine_sample,"osr_config_fps":30,"tabs":a.browser.tabs.len(),"only_one_visible_after_each_switch":a.browser.tabs.values().filter(|t|t.visible).count()==1});
    if let Some(out) = std::env::var_os("CODISSEU_BROWSER_LIBRARY_METRICS") {
        fs::write(out, serde_json::to_vec_pretty(&metrics).unwrap()).unwrap();
    }
    for id in 1..=8 {
        a.dispatch(Command::Browser(browser::Command::Visible {
            id,
            surface: browser::Surface::Page,
            visible: false,
        }))
        .unwrap();
    }
    let quiet_end = Instant::now() + Duration::from_millis(250);
    while Instant::now() < quiet_end {
        a.poll();
        thread::sleep(Duration::from_millis(3));
    }
    assert!(a.browser.frames.is_empty());
    a.shutdown_browser();
    wait(&mut a, |a| a.browser.state == browser::EngineState::Stopped);
    assert!(
        children
            .iter()
            .all(|pid| !PathBuf::from(format!("/proc/{pid}")).exists())
    );
    assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
    drop(a);
    let mut a = AppService::new(path);
    wait(&mut a, |a| !a.is_busy());
    assert_eq!(a.browser_library.state.bookmarks.len(), 1);
    assert_eq!(a.browser_library.state.logins[0].id, key);
    assert_eq!(a.browser.state, browser::EngineState::Stopped);
    open(&mut a, 1, &url);
    eval(&mut a, 1, "document.getElementById('p').value='';true");
    a.fill_browser_login(1, &key).unwrap();
    wait(&mut a, |a| !a.browser_login_busy());
    assert_eq!(
        eval(
            &mut a,
            1,
            "document.getElementById('p').value==='owned-fixture-only'"
        ),
        json!(true)
    );
    library(&mut a, Action::DeleteLogin(key));
    assert!(a.browser_library.state.logins.is_empty());
    a.shutdown_browser();
    wait(&mut a, |a| a.browser.state == browser::EngineState::Stopped);
    assert!(
        !fs::read_to_string(dir.path().join("session.browser-library.json"))
            .unwrap()
            .contains("owned-fixture-only")
    );
}

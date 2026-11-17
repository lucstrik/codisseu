//! Opt-in integration check: official server, its own login, only temporary samples.
use codisseu_ai::copilot::{Client, Document, default_program, file_uri, language, position};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn main() {
    let mut c = Client::start(default_program(), Vec::new()).expect("Copilot server");
    let until = Instant::now() + Duration::from_secs(600);
    let mut asked = false;
    let mut code = None;
    let mut next = 0;
    let mut pending = false;
    let mut received = 0;
    let cases = [
        (
            "php",
            "<?php\n// Return the sum of two integers.\nfunction sum(int $a, int $b): int {\n    ",
        ),
        (
            "sql",
            "-- Select the id and name of every active user from the users table.\nSELECT ",
        ),
        (
            "md",
            "# Getting started\n\nInstall the package using npm:\n\n```bash\n",
        ),
        (
            "js",
            "// Return the sum of two numbers.\nfunction sum(a, b) {\n  ",
        ),
        (
            "ts",
            "// Return the sum of two numbers.\nfunction sum(a: number, b: number): number {\n  ",
        ),
        ("unknown", "The days of the week are Monday, Tuesday, "),
    ];
    let fixture = tempfile::tempdir().unwrap();
    while Instant::now() < until {
        c.poll();
        if c.ready && !asked {
            c.sign_in().unwrap();
            asked = true;
        }
        if let Some(login) = c.login.as_ref()
            && code.as_ref() != Some(&login.code)
        {
            println!(
                "{}",
                serde_json::json!({"event":"device_code","code":login.code,"url":"https://github.com/login/device"})
            );
            code = Some(login.code.clone());
            c.finish_login().unwrap();
        }
        if let Some((id, url)) = c.open_url.take() {
            println!("{}", serde_json::json!({"event":"open_url","url":url}));
            c.reply(id, serde_json::json!({"success":true}));
        }
        for m in std::mem::take(&mut c.messages) {
            println!(
                "{}",
                serde_json::json!({"event":"message","text":m.text,"actions":m.actions})
            );
            c.reply(m.id, serde_json::Value::Null);
        }
        if pending && !c.busy {
            let got = c.suggestion.is_some();
            received += usize::from(got);
            println!(
                "{}",
                serde_json::json!({"event":"result","extension":cases[next-1].0,"received":got,"text":c.suggestion.as_ref().map(|s|s.suffix.chars().take(512).collect::<String>()),"error":c.error})
            );
            pending = false;
            c.cancel();
        }
        if c.authenticated && !pending {
            if next == cases.len() {
                println!(
                    "{}",
                    serde_json::json!({"event":"finished","received":received,"requested":cases.len()})
                );
                return;
            }
            let (ext, text) = cases[next];
            let path = fixture.path().join(format!("sample.{ext}"));
            std::fs::write(&path, text).unwrap();
            c.complete(
                Document {
                    uri: file_uri(&path).unwrap(),
                    version: next as u64 + 1,
                    language: language(&path).into(),
                    text: Arc::from(text),
                    position: position(text, text.chars().count()).unwrap(),
                },
                true,
            )
            .unwrap();
            next += 1;
            pending = true;
        }
        if !pending && let Some(e) = c.error.take() {
            println!("{}", serde_json::json!({"event":"error","message":e}));
            if !c.ready {
                std::process::exit(2)
            }
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    eprintln!("Copilot check timed out");
    std::process::exit(3)
}

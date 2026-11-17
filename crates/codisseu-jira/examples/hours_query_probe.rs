//! Explicit diagnosis, bounded read-only queries; no token, names or keys in output.
use codisseu_jira::{Profile, Secret, period::Period};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
fn main() -> Result<(), String> {
    let source = std::env::var_os("CODISSEU_JIRA_PROFILE_FILE").ok_or("Profile absent")?;
    let profile: Profile =
        serde_json::from_slice(&std::fs::read(source).map_err(|_| "Profile inaccessible")?)
            .map_err(|_| "Invalid profile")?;
    let mut c = profile.connection;
    c.validate()?;
    let id = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(1, &c.site, &c.api, &c.email)).map_err(|_| "Invalid identity")?
        )
    );
    let token = Secret::new(
        keyring::Entry::new("io.codisseu.Codisseu.jira.v1", &id)
            .map_err(|_| "Vault inaccessible")?
            .get_password()
            .map_err(|_| "Saved token inaccessible")?,
    );
    let client = reqwest::Client::builder()
        .retry(reqwest::retry::never())
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|_| "Client unavailable")?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Runtime unavailable")?;
    let (from, to) = Period::default().dates()?;
    let base = format!(
        "worklogAuthor = currentUser() AND worklogDate >= \"{from}\" AND worklogDate < \"{to}\""
    );
    rt.block_on(async {
        for (name, jql) in [
            ("month-updated-order", format!("{base} ORDER BY updated DESC")),
            ("month-no-order", base.clone()),
            ("month-key-order", format!("{base} ORDER BY key ASC")),
            ("month-dates-first", format!("worklogDate >= \"{from}\" AND worklogDate < \"{to}\" AND worklogAuthor = currentUser()")),
            ("current-day", {let p=Period {mode:codisseu_jira::period::Mode::Day,..Period::default()};let(a,b)=p.dates()?;format!("worklogAuthor = currentUser() AND worklogDate >= \"{a}\" AND worklogDate < \"{b}\"")}),
        ] {
            let start = Instant::now();
            let result=client.post(format!("{}/rest/api/3/search/jql",c.api)).basic_auth(&c.email,Some(token.expose())).json(&serde_json::json!({"jql":jql,"maxResults":50,"fields":["summary"]})).send().await;
            let mut record=serde_json::json!({"variant":name,"elapsed_ms":start.elapsed().as_millis(),"remote_writes":0});
            match result {
                Ok(r)=>{record["status"]=r.status().as_u16().into(); if r.status().is_success(){let v:serde_json::Value=r.json().await.map_err(|_| "Invalid response")?;record["page_items"]=v["issues"].as_array().map_or(0,Vec::len).into();record["is_last"]=v["isLast"].clone();}},
                Err(e)=>{record["timeout"]=e.is_timeout().into();}
            }
            record["elapsed_ms"] = (start.elapsed().as_millis() as u64).into();
            println!("{record}");
        }
        Ok::<(),String>(())
    })
}

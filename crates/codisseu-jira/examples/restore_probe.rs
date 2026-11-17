//! Explicit read-only acceptance: copied profile + existing OS vault, counts only.
use codisseu_jira::Service;
use std::time::{Duration, Instant};

fn main() -> Result<(), String> {
    let source = std::env::var_os("CODISSEU_JIRA_PROFILE_FILE").ok_or("Profile file absent")?;
    let dir = tempfile::tempdir().map_err(|_| "Temporary profile unavailable")?;
    let path = dir.path().join("jira.json");
    std::fs::copy(source, &path).map_err(|_| "Profile inaccessible")?;
    let mut service = Service::new(path);
    let deadline = Instant::now() + Duration::from_secs(90);
    while service.main_busy() {
        service.poll();
        if Instant::now() >= deadline {
            return Err("Restore acceptance timed out".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    println!(
        "{}",
        serde_json::json!({
            "kind":"automatic Jira restore from existing vault; read-only",
            "loaded":service.loaded,
            "connected":service.connected,
            "identity_present":service.identity.is_some(),
            "page_items":service.page.issues.len(),
            "error_present":service.error.is_some(),
            "manual_connect_calls":0,
            "supplied_token":false,
            "remote_writes":0,
            "personal_profile_modified":false
        })
    );
    if !service.connected || service.error.is_some() {
        return Err("Automatic Jira restore failed".into());
    }
    if std::env::var_os("CODISSEU_JIRA_PROBE_CURRENT_HOURS").is_some() {
        let started = Instant::now();
        service
            .timesheet
            .start(codisseu_jira::period::Period::default())?;
        while service.timesheet.busy {
            service.poll();
            if started.elapsed() > Duration::from_secs(40) {
                service.timesheet.cancel();
                return Err("Current hours read exceeded the acceptance deadline".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let view = service
            .timesheet
            .view
            .as_ref()
            .ok_or("Missing hours result")?;
        println!(
            "{}",
            serde_json::json!({"kind":"current month personal hours; read-only", "state":format!("{:?}",view.state), "elapsed_ms":started.elapsed().as_millis(), "requests":view.requests,"inspected":view.inspected,"own_records":view.entries.len(),"error_present":service.timesheet.error.is_some(),"remote_writes":0,"personal_profile_modified":false})
        );
        if service.timesheet.error.is_some() {
            return Err("Current hours read failed".into());
        }
    }
    Ok(())
}

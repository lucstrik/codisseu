//! Live read-only acceptance. Prints counts only, never names, keys or worklog content.
use codisseu_jira::{Connection, Secret, client::JiraClient, period::Period, timesheet};
fn main() -> Result<(), String> {
    let file = std::env::var_os("CODISSEU_JIRA_TOKEN_FILE").ok_or("Token file absent")?;
    let token = Secret::new(
        std::fs::read_to_string(file)
            .map_err(|_| "Token inaccessible")?
            .trim()
            .into(),
    );
    let connection = Connection {
        site: std::env::var("CODISSEU_JIRA_SITE").map_err(|_| "Site absent")?,
        email: std::env::var("CODISSEU_JIRA_EMAIL").map_err(|_| "Email absent")?,
        api: String::new(),
    };
    let mut client = JiraClient::new(connection, token)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Runtime unavailable")?;
    let mut period = Period::default();
    if let Ok(date) = std::env::var("CODISSEU_JIRA_REPORT_DATE") {
        period.anchor = date;
        period.validate()?;
    }
    let view = rt.block_on(timesheet::read_personal(&mut client, period, 14))?;
    println!(
        "{}",
        serde_json::json!({"kind":"personal worklogs live read-only bounded scan","state":format!("{:?}",view.state),"requests":view.requests,"inspected":view.inspected,"own_records":view.entries.len(),"own_days":view.days.len(),"own_cards":view.cards,"error":view.message,"writes":0,"credential_storage":false})
    );
    if view.state == timesheet::State::Error {
        return Err("Personal hours probe failed".into());
    }
    Ok(())
}

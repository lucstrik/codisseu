//! Explicit live read-only acceptance; never imports credentials into the product.
use codisseu_jira::{client::JiraClient, *};
fn main() -> Result<(), String> {
    let file = std::env::var_os("CODISSEU_JIRA_TOKEN_FILE")
        .ok_or("Defina CODISSEU_JIRA_TOKEN_FILE para uma credencial autorizada")?;
    let token = Secret::new(
        std::fs::read_to_string(file)
            .map_err(|_| "Token inacessível")?
            .trim()
            .into(),
    );
    let c = Connection {
        site: std::env::var("CODISSEU_JIRA_SITE").map_err(|_| "Site ausente")?,
        email: std::env::var("CODISSEU_JIRA_EMAIL").map_err(|_| "Email ausente")?,
        api: std::env::var("CODISSEU_JIRA_API").unwrap_or_default(),
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Runtime indisponível")?;
    rt.block_on(probe(JiraClient::new(c, token)?))
}
async fn probe(mut client: JiraClient) -> Result<(), String> {
    let id = client.connect().await?;
    let avatar = if id.avatar.is_empty() {
        false
    } else {
        client.image(&id.avatar).await.is_ok()
    };
    let page = client.search(&Filters::default(), None).await?;
    let mut next_count = None;
    if let Some(cursor) = page.next.clone() {
        next_count = Some(
            client
                .search(&Filters::default(), Some(cursor))
                .await?
                .issues
                .len(),
        )
    }
    let mut detail = false;
    let mut comments = 0;
    let mut hours = 0;
    let mut adf_or_markdown = false;
    let mut relations = 0;
    if let Some(issue) = page.issues.first() {
        let d = client.detail(&issue.key).await?;
        detail = true;
        adf_or_markdown = !d.issue.description.is_empty();
        relations = d.issue.relations.len();
        comments = client.comments(&issue.key, 0).await?.0.len();
        hours = client.worklogs(&issue.key, 0).await?.0.len()
    }
    let mut image_attachment = false;
    for issue in page.issues.iter().skip(1).take(5) {
        let d = client.detail(&issue.key).await?;
        adf_or_markdown |= !d.issue.description.is_empty();
        if let Some(a) = d
            .issue
            .attachments
            .iter()
            .find(|a| a.mime.starts_with("image/"))
        {
            let url = if a.thumbnail.is_empty() {
                &a.url
            } else {
                &a.thumbnail
            };
            image_attachment = client.image(url).await.is_ok();
            if adf_or_markdown {
                break;
            }
        }
    }
    if let Ok(key) = std::env::var("CODISSEU_JIRA_PROBE_KEY") {
        let d = client.detail(&key).await?;
        adf_or_markdown |= !d.issue.description.is_empty();
        if let Some(a) = d
            .issue
            .attachments
            .iter()
            .find(|a| a.mime.starts_with("image/"))
        {
            let url = if a.thumbnail.is_empty() {
                &a.url
            } else {
                &a.thumbnail
            };
            image_attachment = match client.image(url).await {
                Ok(_) => true,
                Err(e) => {
                    eprintln!("Image probe: {e}");
                    false
                }
            };
        }
    }
    let mut done = Filters::default();
    done.status = Status::Done;
    let completed = client.search(&done, None).await?;
    let projects = client.suggestions(Picker::Project, "").await?;
    let users = client.suggestions(Picker::User, "Lucas").await?;
    let kinds = client.suggestions(Picker::Kind, "").await?;
    let parents = client.suggestions(Picker::Parent, "").await?;
    let teams = if id.team_field.is_some() {
        Some(client.suggestions(Picker::Team, "").await.map(|v| v.len()))
    } else {
        None
    };
    println!("{}",serde_json::to_string_pretty(&serde_json::json!({
"kind":"live read-only Jira Cloud REST v3","authenticated":true,"active_page_items":page.issues.len(),"has_next":page.next.is_some(),"next_page_items":next_count,"completed_page_items":completed.issues.len(),"detail":detail,"description":adf_or_markdown,"relations":relations,"comment_page_items":comments,"worklog_page_items":hours,"project_suggestions":projects.len(),"user_suggestions":users.len(),"type_suggestions":kinds.len(),"parent_suggestions":parents.len(),"team_field":id.team_field.is_some(),"team_suggestions":teams,"avatar_loaded":avatar,"image_attachment_loaded":image_attachment,"writes":0,"credential_storage":false
}
)).unwrap());
    Ok(())
}

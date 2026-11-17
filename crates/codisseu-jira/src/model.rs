use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    #[default]
    Open,
    Progress,
    Done,
    All,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Em aberto",
            Self::Progress => "Em progresso",
            Self::Done => "Concluídas",
            Self::All => "Todos os status",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Filters {
    pub created_month: Option<String>,
    pub text: String,
    pub mine: bool,
    pub assignee: Option<Choice>,
    pub project: Option<Choice>,
    pub kind: Option<Choice>,
    pub parent: Option<Choice>,
    pub team: Option<Choice>,
    pub team_field: Option<Choice>,
    pub status: Status,
}
impl Default for Filters {
    fn default() -> Self {
        Self {
            created_month: None,
            text: String::new(),
            mine: true,
            assignee: None,
            project: None,
            kind: None,
            parent: None,
            team: None,
            team_field: None,
            status: Status::Open,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct View {
    pub name: String,
    pub filters: Filters,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    pub site: String,
    pub email: String,
    pub api: String,
}
impl Connection {
    pub fn validate(&mut self) -> Result<(), String> {
        self.email = self.email.trim().to_string();
        self.site = base(&self.site, false)?;
        self.api = if self.api.trim().is_empty() {
            self.site.clone()
        } else {
            base(&self.api, true)?
        };
        if self.email.len() > 320
            || !self.email.contains('@')
            || self.email.chars().any(char::is_control)
        {
            return Err("Informe o email da conta Atlassian".into());
        }
        if self.api != self.site {
            let u = url::Url::parse(&self.api).map_err(|_| "URL da API inválida")?;
            let segments: Vec<_> = u.path().trim_matches('/').split('/').collect();
            if u.host_str() != Some("api.atlassian.com")
                || segments.len() != 3
                || segments[0] != "ex"
                || segments[1] != "jira"
                || segments[2].is_empty()
            {
                return Err("Gateway deve ser https://api.atlassian.com/ex/jira/CLOUD_ID".into());
            }
        }
        Ok(())
    }
}
fn base(s: &str, path: bool) -> Result<String, String> {
    let u = url::Url::parse(s.trim()).map_err(|_| "Informe uma URL HTTPS válida do Jira Cloud")?;
    if u.scheme() != "https"
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
        || (!path && u.path() != "/" && !u.path().is_empty())
    {
        return Err("Use HTTPS sem usuário, senha, query ou fragmento na URL".into());
    }
    Ok(u.as_str().trim_end_matches('/').into())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub timesheet: crate::period::Period,
    pub version: u32,
    pub enabled: bool,
    /// Legacy profiles may already have a credential in the OS vault.
    pub auto_connect: bool,
    pub connection: Connection,
    pub filters: Filters,
    pub views: Vec<View>,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            timesheet: crate::period::Period::default(),
            version: 1,
            enabled: false,
            auto_connect: true,
            connection: Connection::default(),
            filters: Filters::default(),
            views: vec![],
        }
    }
}
impl Profile {
    pub fn validate(&self) -> Result<(), String> {
        self.timesheet.validate()?;
        if self.version != 1
            || self.views.len() > 20
            || self.filters.text.len() > 512
            || self.views.iter().any(|v| {
                v.name.trim().is_empty() || v.name.len() > 80 || v.filters.text.len() > 512
            })
        {
            return Err(
                "Preferências Jira inválidas ou acima do limite; arquivo preservado".into(),
            );
        }
        for f in std::iter::once(&self.filters).chain(self.views.iter().map(|v| &v.filters)) {
            if let Some(month) = &f.created_month {
                crate::period::month_bounds(month)?;
            }
            for c in [
                &f.assignee,
                &f.project,
                &f.kind,
                &f.parent,
                &f.team,
                &f.team_field,
            ]
            .into_iter()
            .flatten()
            {
                if c.id.len() > 256 || c.name.len() > 512 || c.id.chars().any(char::is_control) {
                    return Err("Filtro Jira inválido".into());
                }
            }
        }
        if !self.connection.site.is_empty() {
            self.connection.clone().validate()?;
        }
        Ok(())
    }
}
pub fn quote(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace(['\r', '\n', '\t'], " ")
    )
}
pub fn issue_key(s: &str) -> bool {
    let Some((a, b)) = s.split_once('-') else {
        return false;
    };
    !a.is_empty()
        && a.len() < 64
        && a.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && !b.is_empty()
        && b.len() < 20
        && b.chars().all(|c| c.is_ascii_digit())
}
impl Filters {
    pub fn jql(&self, team_field: Option<&str>) -> Result<String, String> {
        let mut clauses = vec![];
        if let Some(month) = &self.created_month {
            let (start, end) = crate::period::month_bounds(month)?;
            clauses.push(format!(
                "created >= {} AND created < {}",
                quote(&start),
                quote(&end)
            ));
        }
        if self.mine {
            clauses.push("assignee = currentUser()".into())
        } else if let Some(c) = &self.assignee {
            clauses.push(format!("assignee = {}", quote(&c.id)))
        }
        if let Some(c) = &self.project {
            clauses.push(format!("project = {}", quote(&c.id)))
        }
        if let Some(c) = &self.kind {
            clauses.push(format!("issuetype = {}", quote(&c.id)))
        }
        if let Some(c) = &self.parent {
            clauses.push(format!("parent = {}", quote(&c.id)))
        }
        if let Some(c) = &self.team {
            let f = self
                .team_field
                .as_ref()
                .map(|f| f.id.as_str())
                .or(team_field)
                .ok_or("Este Jira não disponibilizou um campo de equipe pesquisável")?;
            clauses.push(format!("{} = {}", custom_jql(f)?, quote(&c.id)));
        }
        match self.status {
            Status::Open => clauses.push("statusCategory != Done".into()),
            Status::Progress => clauses.push("statusCategory = \"In Progress\"".into()),
            Status::Done => clauses.push("statusCategory = Done".into()),
            Status::All => (),
        }
        let text = self.text.trim();
        if !text.is_empty() {
            if issue_key(&text.to_uppercase()) {
                clauses.push(format!("key = {}", quote(&text.to_uppercase())))
            } else {
                let literal = lucene(text);
                clauses.push(format!("text ~ {}", quote(&format!("\"{literal}\""))));
            }
        }
        // Enhanced search requires a bounded query, including in the all/mine-off view.
        if clauses.is_empty() {
            clauses.push("created >= \"1970-01-01\"".into())
        }
        Ok(format!(
            "{} ORDER BY updated DESC, key ASC",
            clauses.join(" AND ")
        ))
    }
}
pub fn custom_jql(f: &str) -> Result<String, String> {
    let id = f
        .strip_prefix("customfield_")
        .ok_or("Campo de equipe inválido")?;
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err("Campo de equipe inválido".into());
    }
    Ok(format!("cf[{id}]"))
}
pub fn lucene(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        if "+-&|!(){}[]^\"~*?:\\/".contains(c) {
            o.push('\\')
        }
        if !c.is_control() {
            o.push(c)
        }
    }
    o
}

#[derive(Clone, Debug, Default)]
pub struct Issue {
    pub key: String,
    pub title: String,
    pub status: String,
    pub category: String,
    pub kind: String,
    pub project: String,
    pub assignee: Choice,
    pub assignee_avatar: String,
    pub reporter: Choice,
    pub reporter_avatar: String,
    pub priority: String,
    pub updated: String,
    pub description: String,
    pub relations: Vec<Relation>,
    pub team: String,
    pub spent: i64,
    pub estimate: i64,
    pub attachments: Vec<Attachment>,
}
#[derive(Clone, Debug)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub url: String,
    pub thumbnail: String,
}
pub fn avatar_url(v: &Value) -> String {
    v["avatarUrls"]["48x48"]
        .as_str()
        .or_else(|| v["avatarUrls"]["32x32"].as_str())
        .unwrap_or_default()
        .into()
}
#[derive(Clone, Debug)]
pub struct Relation {
    pub key: String,
    pub title: String,
    pub label: String,
}
pub fn string(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().into()
}
fn person(v: &Value) -> Choice {
    Choice {
        id: string(v, "accountId"),
        name: string(v, "displayName"),
    }
}
impl Issue {
    pub fn from_json(v: &Value, team: Option<&str>) -> Self {
        let f = &v["fields"];
        let mut relations = vec![];
        if let Some(k) = f["parent"]["key"].as_str() {
            relations.push(Relation {
                key: k.into(),
                title: string(&f["parent"]["fields"], "summary"),
                label: "Pai / história / épico".into(),
            })
        }
        if let Some(sub) = f["subtasks"].as_array() {
            for s in sub.iter().take(100) {
                relations.push(Relation {
                    key: string(s, "key"),
                    title: string(&s["fields"], "summary"),
                    label: "Subtarefa".into(),
                })
            }
        }
        if let Some(links) = f["issuelinks"].as_array() {
            for l in links.iter().take(100) {
                for (a, b) in [("inwardIssue", "inward"), ("outwardIssue", "outward")] {
                    if l[a].is_object() {
                        relations.push(Relation {
                            key: string(&l[a], "key"),
                            title: string(&l[a]["fields"], "summary"),
                            label: string(&l["type"], b),
                        })
                    }
                }
            }
        }
        Self {
            key: string(v, "key"),
            title: string(f, "summary"),
            status: string(&f["status"], "name"),
            category: string(&f["status"]["statusCategory"], "key"),
            kind: string(&f["issuetype"], "name"),
            project: string(&f["project"], "name"),
            assignee: person(&f["assignee"]),
            assignee_avatar: avatar_url(&f["assignee"]),
            reporter: person(&f["reporter"]),
            reporter_avatar: avatar_url(&f["reporter"]),
            priority: string(&f["priority"], "name"),
            updated: string(f, "updated"),
            description: crate::content::markdown(&f["description"]),
            relations,
            team: team.map(|t| string(&f[t], "name")).unwrap_or_default(),
            spent: f["timetracking"]["timeSpentSeconds"]
                .as_i64()
                .unwrap_or_default(),
            estimate: f["timetracking"]["originalEstimateSeconds"]
                .as_i64()
                .unwrap_or_default(),
            attachments: f["attachment"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .take(64)
                        .map(|v| Attachment {
                            id: string(v, "id"),
                            name: string(v, "filename"),
                            mime: string(v, "mimeType"),
                            url: string(v, "content"),
                            thumbnail: string(v, "thumbnail"),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Comment {
    pub author: String,
    pub avatar: String,
    pub created: String,
    pub body: String,
}
#[derive(Clone, Debug)]
pub struct Worklog {
    pub account: String,
    pub author: String,
    pub avatar: String,
    pub started: String,
    pub seconds: i64,
    pub body: String,
}
#[derive(Clone, Debug, Default)]
pub struct Page {
    pub issues: Vec<Issue>,
    pub next: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub struct Detail {
    pub issue: Issue,
    pub comments: Vec<Comment>,
    pub comment_total: usize,
    pub comment_offset: usize,
    pub worklogs: Vec<Worklog>,
    pub worklog_total: usize,
    pub worklog_offset: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picker {
    User,
    Project,
    Kind,
    Parent,
    Team,
}
impl Picker {
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "Responsável",
            Self::Project => "Projeto",
            Self::Kind => "Tipo",
            Self::Parent => "História / épico",
            Self::Team => "Equipe",
        }
    }
}

#[derive(Clone, Debug)]
pub struct WorklogDraft {
    pub key: String,
    pub minutes: u32,
    pub started: String,
    pub comment: String,
}
impl WorklogDraft {
    pub fn validate(&self) -> Result<(), String> {
        if !issue_key(&self.key)
            || !(1..=1440).contains(&self.minutes)
            || self.comment.len() > 16384
            || chrono::DateTime::parse_from_rfc3339(&self.started).is_err()
        {
            return Err(
                "Informe de1a1440minutos e início ISO8601 com fuso (ex.:2026-11-03T09:00:00-03:00)"
                    .into(),
            );
        }
        Ok(())
    }
    pub fn body(&self) -> Result<Value, String> {
        self.validate()?;
        let d = chrono::DateTime::parse_from_rfc3339(&self.started).map_err(|_| "Data inválida")?;
        Ok(
            serde_json::json!({"timeSpentSeconds":self.minutes*60,"started":d.format("%Y-%m-%dT%H:%M:%S%.3f%z").to_string(),"comment":{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":if self.comment.trim().is_empty(){"Apontamento de trabalho"}else{&self.comment}}]}]}}),
        )
    }
}

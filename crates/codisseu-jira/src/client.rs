use crate::{Secret, content, model::*};
use reqwest::{Client, Method};
use serde_json::{Value, json};
use std::time::Duration;

pub struct JiraClient {
    http: Client,
    pub connection: Connection,
    token: Secret,
    pub team_field: Option<String>,
    pub(crate) default_team_field: Option<String>,
    cooldown: std::sync::Arc<std::sync::Mutex<Option<std::time::Instant>>>,
}
#[derive(Clone, Debug)]
pub struct Identity {
    pub name: String,
    pub account: String,
    pub avatar: String,
    pub team_field: Option<String>,
    pub team_fields: Vec<Choice>,
}
impl JiraClient {
    pub(crate) fn fork(&self) -> Result<Self, String> {
        // Each worker drives its own current-thread runtime. Sharing reqwest's
        // pool would reuse sockets whose reactor sleeps in the other worker's
        // blocking recv(), stalling the report until another main command runs.
        let mut client = Self::build(
            self.connection.clone(),
            Secret::new(self.token.expose().into()),
        )?;
        client.team_field = self.team_field.clone();
        client.default_team_field = self.default_team_field.clone();
        // Rate limits belong to the account, unlike HTTP socket ownership.
        client.cooldown = self.cooldown.clone();
        Ok(client)
    }
    pub async fn image(&self, url: &str) -> Result<crate::media::Raster, String> {
        self.media_client()?.fetch(url).await
    }
    pub(crate) fn media_client(&self) -> Result<crate::media::ImageClient, String> {
        crate::media::ImageClient::new(
            self.connection.clone(),
            Secret::new(self.token.expose().into()),
        )
    }
    pub(crate) fn token_value(&self) -> &str {
        self.token.expose()
    }
    pub fn new(mut connection: Connection, token: Secret) -> Result<Self, String> {
        connection.validate()?;
        if token.expose().trim().is_empty() || token.expose().len() > 16384 {
            return Err("Informe o token de API".into());
        }
        Self::build(connection, token)
    }
    fn build(connection: Connection, token: Secret) -> Result<Self, String> {
        let http = Client::builder()
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(25))
            .user_agent("Codisseu/0.1 Jira")
            .build()
            .map_err(|_| "Não foi possível iniciar o cliente HTTPS")?;
        Ok(Self {
            http,
            connection,
            token,
            team_field: None,
            default_team_field: None,
            cooldown: std::sync::Arc::new(std::sync::Mutex::new(None)),
        })
    }
    #[cfg(test)]
    pub(crate) fn fixture(base: String) -> Self {
        Self::build(
            Connection {
                site: base.clone(),
                api: base,
                email: "fixture@example.invalid".into(),
            },
            Secret::new("fixture-token".into()),
        )
        .unwrap()
    }
    #[cfg(test)]
    pub(crate) fn fixture_timeout(base: String, timeout: Duration) -> Self {
        let mut c = Self::fixture(base);
        c.http = Client::builder()
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .build()
            .unwrap();
        c
    }
    async fn request(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
        write: bool,
    ) -> Result<Value, String> {
        if let Ok(c) = self.cooldown.lock()
            && let Some(until) = *c
            && until > std::time::Instant::now()
        {
            return Err(format!(
                "Limite do Jira: aguarde{}s antes de consultar novamente.",
                until
                    .saturating_duration_since(std::time::Instant::now())
                    .as_secs()
                    + 1
            ));
        }
        let url = format!("{}/rest/api/3/{path}", self.connection.api);
        let mut req = self
            .http
            .request(method, url)
            .basic_auth(&self.connection.email, Some(self.token.expose()))
            .header("Accept", "application/json")
            .query(query);
        if let Some(body) = body {
            req = req.json(&body)
        }
        let mut r=req.send().await.map_err(|e| -> String { if write{"A resposta do apontamento não chegou. Ele pode ter sido gravado: confira os registros no Jira antes de reenviar.".into()}else if e.is_timeout(){"O Jira excedeu25s. Verifique a conexão e tente novamente.".into()}else{"Não foi possível alcançar o Jira. Verifique rede, TLS e endereço.".into()} })?;
        let status = r.status();
        if status.as_u16() == 429 {
            let secs = r
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(60)
                .clamp(1, 3600);
            if let Ok(mut c) = self.cooldown.lock() {
                *c = Some(std::time::Instant::now() + Duration::from_secs(secs));
            }
        }
        if !status.is_success() {
            return Err(match status.as_u16(){
            401=>"Token inválido/expirado ou email incorreto. Revise a conexão.".into(),403=>"Sua conta/token não tem permissão para esta operação no Jira.".into(),404=>"Card/recurso não encontrado ou sem permissão de leitura.".into(),429=>{let sec=r.headers().get("Retry-After").and_then(|v|v.to_str().ok()).and_then(|s|s.parse::<u64>().ok()).unwrap_or(60).clamp(1,3600);format!("Limite do Jira atingido. Aguarde{sec}s antes de atualizar.")},300..=399=>"Jira redirecionou a requisição. Revise o endereço; o token não foi encaminhado.".into(),400=>"O Jira recusou os filtros/campos ou o apontamento. Revise os valores e permissões.".into(),_ if write=>"O Jira falhou ao responder. Confira se o apontamento foi gravado antes de reenviar.".into(),_=>format!("Jira temporariamente indisponível (HTTP{}). Tente atualizar depois.",status.as_u16())});
        }
        const MAX: usize = 8 * 1024 * 1024;
        if r.content_length().is_some_and(|l| l > MAX as u64) {
            return Err("Resposta Jira acima de8MiB; reduza o escopo".into());
        }
        let mut bytes = Vec::new();
        while let Some(c) = r.chunk().await.map_err(|_| {
            if write {
                "Resposta interrompida; confira os registros no Jira antes de reenviar"
            } else {
                "Resposta do Jira interrompida"
            }
        })? {
            if bytes.len() + c.len() > MAX {
                return Err("Resposta Jira acima de8MiB".into());
            }
            bytes.extend_from_slice(&c)
        }
        serde_json::from_slice(&bytes).map_err(|_| {
            if write {
                "Resposta inválida; confira os registros no Jira antes de reenviar".into()
            } else {
                "Jira retornou conteúdo inválido em vez de JSON".into()
            }
        })
    }
    pub async fn connect(&mut self) -> Result<Identity, String> {
        let me = self
            .request(Method::GET, "myself", &[], None, false)
            .await?;
        if me["active"] == false || me["accountId"].as_str().is_none_or(|id| id.is_empty()) {
            return Err("A conta Jira não está ativa/autenticada".into());
        }
        // Optional metadata failure must not prevent reading assigned work.
        let mut team_fields = Vec::new();
        if let Ok(fields) = self.request(Method::GET, "field", &[], None, false).await {
            if let Some(a) = fields.as_array() {
                for f in a {
                    let name = string(f, "name");
                    let lower = name.to_lowercase();
                    let kind = string(&f["schema"], "custom");
                    if f["searchable"] != false
                        && (kind == "com.atlassian.teams:rm-teams-custom-field-team"
                            || kind
                                == "com.atlassian.jira.plugin.system.customfieldtypes:atlassian-team"
                            || matches!(lower.as_str(), "team" | "equipe" | "squad" | "tempo team"))
                        && custom_jql(&string(f, "id")).is_ok()
                    {
                        team_fields.push(Choice {
                            id: string(f, "id"),
                            name,
                        })
                    }
                }
            }
            self.team_field = team_fields
                .iter()
                .find(|f| f.name.eq_ignore_ascii_case("team"))
                .or_else(|| team_fields.first())
                .map(|f| f.id.clone());
        }
        self.default_team_field = self.team_field.clone();
        Ok(Identity {
            name: string(&me, "displayName"),
            account: string(&me, "accountId"),
            avatar: avatar_url(&me),
            team_field: self.team_field.clone(),
            team_fields,
        })
    }
    pub async fn search(&self, filters: &Filters, cursor: Option<String>) -> Result<Page, String> {
        let mut body = json!({"jql":filters.jql(self.team_field.as_deref())?,"maxResults":50,"fields":["summary","status","issuetype","project","assignee","priority","updated","parent"]});
        if let Some(c) = cursor {
            if c.len() > 16384 {
                return Err("Cursor Jira inválido".into());
            }
            body["nextPageToken"] = c.into()
        }
        let v = self
            .request(Method::POST, "search/jql", &[], Some(body), false)
            .await?;
        let a = v["issues"]
            .as_array()
            .ok_or("Resposta de busca sem lista de cards")?;
        if a.len() > 50 {
            return Err("Jira excedeu o tamanho de página solicitado".into());
        }
        let next = if v["isLast"] == true {
            None
        } else {
            v["nextPageToken"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 16384)
                .map(str::to_owned)
        };
        Ok(Page {
            issues: a
                .iter()
                .map(|i| Issue::from_json(i, self.team_field.as_deref()))
                .collect(),
            next,
        })
    }
    pub async fn detail(&self, key: &str) -> Result<Detail, String> {
        check_key(key)?;
        let mut fields="summary,description,status,issuetype,project,assignee,reporter,priority,updated,parent,subtasks,issuelinks,timetracking,attachment".to_string();
        if let Some(t) = &self.team_field {
            fields.push(',');
            fields.push_str(t)
        }
        let v = self
            .request(
                Method::GET,
                &format!("issue/{key}"),
                &[("fields", fields)],
                None,
                false,
            )
            .await?;
        let mut issue = Issue::from_json(&v, self.team_field.as_deref());
        for a in &mut issue.attachments {
            if !a.id.is_empty() && a.id.chars().all(|c| c.is_ascii_digit()) {
                a.url = format!(
                    "{}/rest/api/3/attachment/content/{}?redirect=false",
                    self.connection.api, a.id
                );
                if a.mime.starts_with("image/") {
                    a.thumbnail = format!(
                        "{}/rest/api/3/attachment/thumbnail/{}?redirect=false&width=640&height=400",
                        self.connection.api, a.id
                    );
                }
            }
        }
        Ok(Detail {
            issue,
            ..Default::default()
        })
    }
    pub async fn comments(
        &self,
        key: &str,
        offset: usize,
    ) -> Result<(Vec<Comment>, usize), String> {
        check_key(key)?;
        let v = self
            .request(
                Method::GET,
                &format!("issue/{key}/comment"),
                &[
                    ("maxResults", "20".into()),
                    ("startAt", offset.to_string()),
                    ("orderBy", "-created".into()),
                ],
                None,
                false,
            )
            .await?;
        let a = v["comments"]
            .as_array()
            .ok_or("Comentários indisponíveis")?;
        if a.len() > 20 {
            return Err("Comentários excederam o limite da página".into());
        }
        Ok((
            a.iter()
                .map(|c| Comment {
                    author: string(&c["author"], "displayName"),
                    avatar: avatar_url(&c["author"]),
                    created: string(c, "created"),
                    body: content::markdown(&c["body"]),
                })
                .collect(),
            v["total"].as_u64().unwrap_or(0) as usize,
        ))
    }
    pub async fn worklogs(
        &self,
        key: &str,
        offset: usize,
    ) -> Result<(Vec<Worklog>, usize), String> {
        check_key(key)?;
        let v = self
            .request(
                Method::GET,
                &format!("issue/{key}/worklog"),
                &[("maxResults", "20".into()), ("startAt", offset.to_string())],
                None,
                false,
            )
            .await?;
        let a = v["worklogs"].as_array().ok_or("Registros indisponíveis")?;
        if a.len() > 20 {
            return Err("Registros excederam o limite da página".into());
        }
        Ok((
            a.iter()
                .map(|w| Worklog {
                    account: string(&w["author"], "accountId"),
                    author: string(&w["author"], "displayName"),
                    avatar: avatar_url(&w["author"]),
                    started: string(w, "started"),
                    seconds: w["timeSpentSeconds"].as_i64().unwrap_or_default(),
                    body: content::markdown(&w["comment"]),
                })
                .collect(),
            v["total"].as_u64().unwrap_or(0) as usize,
        ))
    }
    pub(crate) async fn timesheet_candidates(
        &self,
        period: &crate::period::Period,
        cursor: Option<String>,
    ) -> Result<(Vec<Value>, Option<String>), String> {
        let (from, to) = period.dates()?;
        let jql = format!(
            "worklogAuthor = currentUser() AND worklogDate >= {} AND worklogDate < {} ORDER BY updated DESC",
            quote(&from.to_string()),
            quote(&to.to_string())
        );
        let mut body = json!({"jql":jql,"maxResults":50,"fields":["summary"]});
        if let Some(cursor) = cursor {
            body["nextPageToken"] = cursor.into();
        }
        let v = self
            .request(Method::POST, "search/jql", &[], Some(body), false)
            .await?;
        let a = v["issues"]
            .as_array()
            .ok_or("Jira retornou uma lista de cards inválida")?;
        if a.len() > 50 {
            return Err("Página de cards excedeu50itens".into());
        }
        let next = if v["isLast"] == true {
            None
        } else {
            v["nextPageToken"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 16384)
                .map(str::to_owned)
        };
        if v["isLast"] != true && next.is_none() {
            return Err("O Jira não informou o próximo cursor; resultado parcial".into());
        }
        Ok((a.clone(), next))
    }
    pub(crate) async fn period_worklogs(
        &self,
        key: &str,
        offset: usize,
        from: i64,
        to: i64,
    ) -> Result<(Vec<Value>, usize), String> {
        check_key(key)?;
        let v = self
            .request(
                Method::GET,
                &format!("issue/{key}/worklog"),
                &[
                    ("maxResults", "100".into()),
                    ("startAt", offset.to_string()),
                    ("startedAfter", (from - 1).to_string()),
                    ("startedBefore", to.to_string()),
                ],
                None,
                false,
            )
            .await?;
        let a = v["worklogs"]
            .as_array()
            .ok_or("Jira retornou apontamentos inválidos")?;
        let total = v["total"]
            .as_u64()
            .ok_or("Jira não informou a paginação de apontamentos")? as usize;
        if a.len() > 100 {
            return Err("Página de apontamentos excedeu100itens".into());
        }
        Ok((a.clone(), total))
    }
    pub async fn post_worklog(&self, draft: &WorklogDraft) -> Result<(), String> {
        let v = self
            .request(
                Method::POST,
                &format!("issue/{}/worklog", draft.key),
                &[("adjustEstimate", "leave".into())],
                Some(draft.body()?),
                true,
            )
            .await?;
        if v["id"].as_str().is_none() {
            return Err("Resposta sem ID; confira os registros no Jira antes de reenviar".into());
        }
        Ok(())
    }
    pub async fn suggestions(&self, picker: Picker, text: &str) -> Result<Vec<Choice>, String> {
        if text.len() > 256 {
            return Err("Busca de filtro acima de256caracteres".into());
        }
        let (path, query) = match picker {
            Picker::User => (
                "user/search".to_string(),
                vec![("query", text.to_string()), ("maxResults", "20".into())],
            ),
            Picker::Project => (
                "project/search".into(),
                vec![("query", text.to_string()), ("maxResults", "20".into())],
            ),
            Picker::Kind => ("issuetype".into(), vec![]),
            Picker::Parent => {
                let jql = if issue_key(&text.to_uppercase()) {
                    format!("key = {}", quote(&text.to_uppercase()))
                } else if text.trim().is_empty() {
                    "issuetype in standardIssueTypes() ORDER BY updated DESC".into()
                } else {
                    format!(
                        "summary ~ {} ORDER BY updated DESC",
                        quote(&format!("\"{}\"", lucene(text)))
                    )
                };
                (
                    "search/jql".into(),
                    vec![
                        ("jql", jql),
                        ("maxResults", "20".into()),
                        ("fields", "summary,issuetype".into()),
                    ],
                )
            }
            Picker::Team => {
                let f = self
                    .team_field
                    .as_deref()
                    .ok_or("Nenhum campo Team disponível nesta conta")?;
                (
                    "jql/autocompletedata/suggestions".into(),
                    vec![("fieldName", custom_jql(f)?), ("fieldValue", text.into())],
                )
            }
        };
        let v = self
            .request(Method::GET, &path, &query, None, false)
            .await?;
        let a = match picker {
            Picker::Project => v["values"].as_array(),
            Picker::Parent => v["issues"].as_array(),
            Picker::Team => v["results"].as_array(),
            _ => v.as_array(),
        }
        .ok_or("Sugestões indisponíveis para este filtro")?;
        Ok(a.iter()
            .filter_map(|v| {
                let (id, name) = match picker {
                    Picker::User => (string(v, "accountId"), string(v, "displayName")),
                    Picker::Project => (
                        string(v, "id"),
                        format!("{} · {}", string(v, "key"), string(v, "name")),
                    ),
                    Picker::Parent => (
                        string(v, "key"),
                        format!("{} · {}", string(v, "key"), string(&v["fields"], "summary")),
                    ),
                    Picker::Team => (string(v, "value"), string(v, "displayName")),
                    Picker::Kind => (string(v, "id"), string(v, "name")),
                };
                if id.is_empty()
                    || (picker == Picker::Kind
                        && !name.to_lowercase().contains(&text.to_lowercase()))
                {
                    None
                } else {
                    Some(Choice {
                        id,
                        name: strip_markup(&name),
                    })
                }
            })
            .take(20)
            .collect())
    }
}
fn strip_markup(s: &str) -> String {
    let mut tag = false;
    s.chars()
        .filter(|c| {
            if *c == '<' {
                tag = true;
                false
            } else if *c == '>' {
                tag = false;
                false
            } else {
                !tag
            }
        })
        .collect()
}
fn check_key(key: &str) -> Result<(), String> {
    if issue_key(key) {
        Ok(())
    } else {
        Err("Chave de card inválida".into())
    }
}

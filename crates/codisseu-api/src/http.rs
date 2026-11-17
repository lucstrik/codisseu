use crate::{ApiRequest, Auth, KeyValue, RequestBody, ScriptResult, Variables, interpolate};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiResponse {
    pub status: u16,
    pub headers: Vec<KeyValue>,
    pub body: Vec<u8>,
    pub elapsed_ms: u64,
    pub size: usize,
    pub truncated: bool,
}
impl ApiResponse {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn pretty_json(&self) -> Option<String> {
        serde_json::from_slice::<serde_json::Value>(&self.body)
            .ok()
            .and_then(|v| serde_json::to_string_pretty(&v).ok())
    }
}
#[derive(Clone, Debug)]
pub struct ExecutionResult {
    pub response: ApiResponse,
    pub script_result: ScriptResult,
    pub script_error: Option<String>,
}
pub struct RequestJob {
    rx: mpsc::Receiver<Result<ExecutionResult, String>>,
    cancel: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}
impl RequestJob {
    pub fn try_result(&mut self) -> Option<Result<ExecutionResult, String>> {
        match self.rx.try_recv() {
            Ok(result) => {
                if self.join.as_ref().is_some_and(|j| j.is_finished())
                    && let Some(join) = self.join.take()
                {
                    let _ = join.join();
                }
                Some(result)
            }
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("Request worker stopped".into())),
            Err(mpsc::TryRecvError::Empty) => None,
        }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn is_finished(&self) -> bool {
        self.join.as_ref().is_none_or(|j| j.is_finished())
    }
}
impl Drop for RequestJob {
    fn drop(&mut self) {
        self.cancel();
        if self.join.as_ref().is_some_and(|j| j.is_finished())
            && let Some(join) = self.join.take()
        {
            let _ = join.join();
        }
    }
}
#[derive(Clone, Default)]
pub struct ApiClient {
    cookies: Arc<reqwest::cookie::Jar>,
}
impl ApiClient {
    pub fn clear_cookies(&mut self) {
        self.cookies = Arc::new(reqwest::cookie::Jar::default());
    }
    pub fn start(
        &self,
        request: ApiRequest,
        variables: Variables,
        scripts_enabled: bool,
    ) -> Result<RequestJob, String> {
        self.start_scoped(request, variables, Variables::new(), scripts_enabled)
    }
    pub fn start_scoped(
        &self,
        request: ApiRequest,
        environment: Variables,
        collection: Variables,
        scripts_enabled: bool,
    ) -> Result<RequestJob, String> {
        self.start_for_workspace(
            request,
            environment,
            collection,
            true,
            scripts_enabled,
            crate::scripts::ScriptContext::default(),
        )
    }
    pub(crate) fn start_for_workspace(
        &self,
        request: ApiRequest,
        environment: Variables,
        collection: Variables,
        environment_available: bool,
        scripts_enabled: bool,
        context: crate::scripts::ScriptContext,
    ) -> Result<RequestJob, String> {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancelled = cancel.clone();
        let cookies = self.cookies.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        let join = thread::Builder::new()
            .name("codisseu-api-http".into())
            .spawn(move || {
                let result = (|| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|_| "Could not initialize HTTP runtime")?;
                    runtime.block_on(execute(
                        request,
                        environment,
                        collection,
                        environment_available,
                        scripts_enabled,
                        cookies,
                        (cancelled, context),
                    ))
                })();
                let _ = tx.send(result);
            })
            .map_err(|_| "Could not start HTTP worker")?;
        Ok(RequestJob {
            rx,
            cancel,
            join: Some(join),
        })
    }
}
/// Builds the final URL and headers once for both HTTP and cURL export.
pub fn resolve_request(
    request: &ApiRequest,
    variables: &Variables,
) -> Result<(url::Url, Vec<KeyValue>), String> {
    let mut url = url::Url::parse(&request.resolved_url_text(variables)?)
        .map_err(|_| "Invalid request URL")?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Only HTTP and HTTPS requests are supported".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URL credentials are unsupported; use Authorization instead".into());
    }
    let mut headers = vec![];
    for h in request
        .headers
        .iter()
        .filter(|h| h.enabled && !h.key.is_empty())
    {
        headers.push(KeyValue {
            key: interpolate(&h.key, variables)?,
            value: interpolate(&h.value, variables)?,
            ..h.clone()
        });
    }
    for p in request
        .params
        .iter()
        .filter(|p| p.enabled && !p.key.is_empty())
    {
        url.query_pairs_mut().append_pair(
            &interpolate(&p.key, variables)?,
            &interpolate(&p.value, variables)?,
        );
    }
    if matches!(&request.auth, Auth::Bearer { .. } | Auth::Basic { .. }) {
        headers.retain(|h| !h.key.eq_ignore_ascii_case("Authorization"));
    }
    match &request.auth {
        Auth::Bearer { token } => headers.push(KeyValue::new(
            "Authorization",
            format!("Bearer {}", interpolate(token, variables)?),
        )),
        Auth::Basic { username, password } => headers.push(KeyValue::new(
            "Authorization",
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(format!(
                    "{}:{}",
                    interpolate(username, variables)?,
                    interpolate(password, variables)?
                ))
            ),
        )),
        Auth::ApiKey {
            key,
            value,
            in_query,
        } => {
            let key = interpolate(key, variables)?;
            let value = interpolate(value, variables)?;
            if *in_query {
                let pairs = url
                    .query_pairs()
                    .filter(|(k, _)| k != &key)
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect::<Vec<_>>();
                url.set_query(None);
                {
                    let mut query = url.query_pairs_mut();
                    for (k, v) in pairs {
                        query.append_pair(&k, &v);
                    }
                    query.append_pair(&key, &value);
                }
            } else {
                headers.retain(|h| !h.key.eq_ignore_ascii_case(&key));
                headers.push(KeyValue::new(key, value));
            }
        }
        _ => {}
    }
    if headers.len() > 256
        || headers
            .iter()
            .map(|h| h.key.len() + h.value.len())
            .sum::<usize>()
            > 64 * 1024
    {
        return Err("Request headers exceed limit".into());
    }
    Ok((url, headers))
}
async fn execute(
    mut request: ApiRequest,
    mut environment: Variables,
    mut collection: Variables,
    environment_available: bool,
    scripts_enabled: bool,
    cookies: Arc<reqwest::cookie::Jar>,
    runtime: (Arc<AtomicBool>, crate::scripts::ScriptContext),
) -> Result<ExecutionResult, String> {
    let (cancel, mut context) = runtime;
    context.request = Some(request.clone());
    let mut scripts = ScriptResult::default();
    if scripts_enabled {
        scripts = crate::scripts::run_script_context(
            &request.pre_request_script,
            &environment,
            &collection,
            environment_available,
            None,
            &cancel,
            &context,
        )?;
        apply_scope(
            &mut environment,
            &scripts.environment_updates,
            &scripts.environment_unsets,
        );
        apply_scope(
            &mut collection,
            &scripts.collection_updates,
            &scripts.collection_unsets,
        );
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Request cancelled".into());
    }
    if scripts.skipped {
        return Ok(ExecutionResult {
            response: ApiResponse {
                status: 0,
                headers: vec![],
                body: vec![],
                elapsed_ms: 0,
                size: 0,
                truncated: false,
            },
            script_result: scripts,
            script_error: None,
        });
    }
    apply_scope(
        &mut context.globals,
        &scripts.global_updates,
        &scripts.global_unsets,
    );
    if scripts_enabled && !request.pre_request_script.trim().is_empty() {
        context.locals = scripts.locals.clone();
    }
    if let Some(headers) = &scripts.request_headers {
        request.headers = headers.clone();
    }
    if let Some(params) = &scripts.request_params {
        request.params = params.clone();
    }
    if let Some(method) = &scripts.request_method {
        request.method = method.clone();
    }
    let mut variables = context.globals.clone();
    variables.extend(collection.clone());
    variables.extend(environment.clone());
    variables.extend(context.iteration_data.clone());
    variables.extend(context.locals.clone());
    context.request = Some(request.clone());
    let (url, headers) = resolve_request(&request, &variables)?;
    let method = reqwest::Method::from_bytes(request.method.as_bytes())
        .map_err(|_| "Invalid HTTP method")?;
    let timeout = Duration::from_millis(request.settings.timeout_ms.clamp(100, 300_000));
    let mut builder = reqwest::Client::builder()
        .danger_accept_invalid_certs(!request.settings.verify_tls)
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(10)))
        .redirect(if request.settings.follow_redirects {
            reqwest::redirect::Policy::limited(10)
        } else {
            reqwest::redirect::Policy::none()
        });
    if request.settings.use_cookies {
        builder = builder.cookie_provider(cookies);
    }
    let client = builder
        .build()
        .map_err(|_| "Could not configure HTTP client")?;
    let mut request_builder = client.request(method, url);
    let has_content_type = headers
        .iter()
        .any(|h| h.key.eq_ignore_ascii_case("Content-Type"));
    for h in headers {
        let name = reqwest::header::HeaderName::from_bytes(h.key.as_bytes())
            .map_err(|_| "Invalid header name")?;
        let value =
            reqwest::header::HeaderValue::from_str(&h.value).map_err(|_| "Invalid header value")?;
        request_builder = request_builder.header(name, value);
    }
    match &request.body {
        RequestBody::None => {}
        RequestBody::Raw { text, content_type } => {
            let body = interpolate(text, &variables)?;
            if !content_type.is_empty() && !has_content_type {
                request_builder =
                    request_builder.header(reqwest::header::CONTENT_TYPE, content_type);
            }
            request_builder = request_builder.body(body);
        }
        RequestBody::Binary(data) => {
            if data.len() > 8 * 1024 * 1024 {
                return Err("Request body exceeds 8 MiB".into());
            }
            request_builder = request_builder.body(data.clone());
        }
        RequestBody::BinaryFile { path } => {
            let bytes = read_upload(path, request.upload_root.as_deref(), &cancel)?;
            request_builder = request_builder.body(bytes);
        }
        RequestBody::UrlEncoded(rows) => {
            let rows = resolved_rows(rows, &variables)?;
            request_builder = request_builder.form(&rows);
        }
        RequestBody::FormData(rows) => {
            if rows.len() > 4096 {
                return Err("Multipart exceeds 4096 fields".into());
            }
            let mut form = reqwest::multipart::Form::new();
            let mut total = 0;
            for row in rows.iter().filter(|r| r.enabled && !r.key.is_empty()) {
                let key = interpolate(&row.key, &variables)?;
                if let Some(path) = &row.file_path {
                    let bytes = read_upload(path, request.upload_root.as_deref(), &cancel)?;
                    total += bytes.len() + key.len();
                    if total > 8 * 1024 * 1024 {
                        return Err("Multipart body exceeds 8 MiB".into());
                    }
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("upload.bin")
                        .to_owned();
                    let part = reqwest::multipart::Part::bytes(bytes)
                        .file_name(name)
                        .mime_str("application/octet-stream")
                        .map_err(|_| "Could not prepare file upload")?;
                    form = form.part(key, part);
                } else {
                    let value = interpolate(&row.value, &variables)?;
                    total += key.len() + value.len();
                    if total > 8 * 1024 * 1024 {
                        return Err("Multipart body exceeds 8 MiB".into());
                    }
                    form = form.text(key, value);
                }
            }
            request_builder = request_builder.multipart(form);
        }
    }
    let started = Instant::now();
    let future = async {
        let mut response = request_builder.send().await.map_err(safe_http_error)?;
        let status = response.status().as_u16();
        let mut headers = vec![];
        let mut header_bytes = 0;
        for (key, value) in response.headers() {
            header_bytes += key.as_str().len() + value.as_bytes().len();
            if headers.len() >= 256 || header_bytes > 64 * 1024 {
                return Err(String::from("Response headers exceed 64 KiB / 256 entries"));
            }
            headers.push(KeyValue::new(
                key.as_str(),
                value.to_str().unwrap_or("[non UTF-8 header]"),
            ));
        }
        let max = request
            .settings
            .max_response_bytes
            .clamp(1024, 16 * 1024 * 1024);
        let mut body = vec![];
        let mut truncated = false;
        while let Some(chunk) = response.chunk().await.map_err(safe_http_error)? {
            let remaining = max - body.len();
            if chunk.len() > remaining {
                body.extend_from_slice(&chunk[..remaining]);
                truncated = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }
        Ok(ApiResponse {
            status,
            headers,
            size: body.len(),
            body,
            elapsed_ms: started.elapsed().as_millis() as u64,
            truncated,
        })
    };
    tokio::pin!(future);
    let response = loop {
        tokio::select! {result=&mut future=>break result?,_=tokio::time::sleep(Duration::from_millis(20))=>{if cancel.load(Ordering::Acquire){return Err("Request cancelled".into());}}}
    };
    let mut script_error = None;
    if scripts_enabled && !response.truncated {
        match crate::scripts::run_script_context(
            &request.post_response_script,
            &environment,
            &collection,
            environment_available,
            Some(&response),
            &cancel,
            &context,
        ) {
            Ok(post) => {
                if post.next_request_set {
                    scripts.next_request_set = true;
                    scripts.next_request = post.next_request.clone();
                }
                for key in post.global_updates.keys() {
                    scripts.global_unsets.retain(|k| k != key);
                }
                scripts.global_updates.extend(post.global_updates);
                scripts.global_unsets.extend(post.global_unsets);
                scripts.locals = post.locals;
                for key in post.environment_updates.keys() {
                    scripts.environment_unsets.retain(|k| k != key);
                }
                for key in post.collection_updates.keys() {
                    scripts.collection_unsets.retain(|k| k != key);
                }
                scripts.environment_updates.extend(post.environment_updates);
                scripts.collection_updates.extend(post.collection_updates);
                scripts.environment_unsets.extend(post.environment_unsets);
                scripts.collection_unsets.extend(post.collection_unsets);
                scripts.logs.extend(post.logs);
                scripts.tests.extend(post.tests);
            }
            Err(error) => script_error = Some(error),
        }
    } else if scripts_enabled
        && response.truncated
        && !request.post_response_script.trim().is_empty()
    {
        script_error = Some("Response truncated; post-response script was not run".into());
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Request cancelled".into());
    }
    Ok(ExecutionResult {
        response,
        script_result: scripts,
        script_error,
    })
}
fn resolved_rows(
    rows: &[KeyValue],
    variables: &Variables,
) -> Result<Vec<(String, String)>, String> {
    if rows.len() > 4096 {
        return Err("Form exceeds 4096 rows".into());
    }
    let mut total = 0;
    let mut result = vec![];
    for row in rows.iter().filter(|r| r.enabled && !r.key.is_empty()) {
        let k = interpolate(&row.key, variables)?;
        let v = interpolate(&row.value, variables)?;
        total += k.len() + v.len();
        if total > 8 * 1024 * 1024 {
            return Err("Form exceeds 8 MiB".into());
        }
        result.push((k, v));
    }
    Ok(result)
}
fn safe_http_error(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "HTTP request timed out".into()
    } else if e.is_connect() {
        "Could not connect (check host, network and TLS certificate)".into()
    } else if e.is_redirect() {
        "Too many redirects".into()
    } else if e.is_body() || e.is_decode() {
        "HTTP response body failed".into()
    } else {
        "HTTP request failed".into()
    }
}

fn apply_scope(scope: &mut Variables, updates: &Variables, unsets: &[String]) {
    scope.extend(updates.clone());
    for key in unsets {
        scope.remove(key);
    }
}

fn read_upload(
    path: &std::path::Path,
    root: Option<&std::path::Path>,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        root.ok_or("Relative upload requires an available project folder")?
            .join(path)
    };
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NONBLOCK);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "Upload file could not be opened")?;
    let metadata = file
        .metadata()
        .map_err(|_| "Upload file metadata unavailable")?;
    if !metadata.is_file() {
        return Err("Upload source must be a regular file".into());
    }
    if metadata.len() > 8 * 1024 * 1024 {
        return Err("Upload file exceeds 8 MiB".into());
    }
    let mut bytes = vec![];
    let mut buffer = [0u8; 65536];
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("Request cancelled".into());
        }
        let size = file
            .read(&mut buffer)
            .map_err(|_| "Could not read upload file")?;
        if size == 0 {
            break;
        }
        if bytes.len() + size > 8 * 1024 * 1024 {
            return Err("Upload file exceeds 8 MiB".into());
        }
        bytes.extend_from_slice(&buffer[..size]);
    }
    Ok(bytes)
}

use crate::{ApiRequest, Auth, KeyValue, RequestBody, Variables, interpolate, resolve_request};

/// Tokenizes POSIX quoting without invoking a shell or expanding commands/variables.
fn words(input: &str) -> Result<Vec<String>, String> {
    if input.len() > 8 * 1024 * 1024 {
        return Err("cURL input exceeds 8 MiB".into());
    }
    let mut words = vec![];
    let mut word = String::new();
    let mut quote = None;
    let mut active = false;
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match quote {
            Some('\'') => {
                if ch == '\'' {
                    quote = None;
                } else {
                    word.push(ch);
                }
            }
            Some('"') => {
                if ch == '"' {
                    quote = None;
                } else if ch == '\\' {
                    match chars.next() {
                        Some('\n') => {}
                        Some(next @ ('"' | '\\' | '$' | '`')) => word.push(next),
                        Some(next) => {
                            word.push('\\');
                            word.push(next);
                        }
                        None => return Err("Incomplete cURL escape".into()),
                    }
                } else {
                    word.push(ch);
                }
            }
            _ => match ch {
                '\'' | '"' => {
                    quote = Some(ch);
                    active = true;
                }
                '\\' => match chars.next() {
                    Some('\n') => {}
                    Some(next) => {
                        word.push(next);
                        active = true;
                    }
                    None => return Err("Incomplete cURL escape".into()),
                },
                c if c.is_whitespace() => {
                    if active {
                        words.push(std::mem::take(&mut word));
                        active = false;
                    }
                }
                ';' | '|' | '&' | '>' | '<' | '`' => {
                    return Err(
                        "Paste only one cURL command; shell operators are not supported".into(),
                    );
                }
                _ => {
                    word.push(ch);
                    active = true;
                }
            },
        }
    }
    if quote.is_some() {
        return Err("Unclosed cURL quote".into());
    }
    if active {
        words.push(word);
    }
    Ok(words)
}
fn value(args: &[String], index: &mut usize, inline: Option<String>) -> Result<String, String> {
    if let Some(v) = inline {
        return Ok(v);
    }
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| "Missing cURL option value".into())
}
fn literal_data(v: String) -> Result<String, String> {
    if v.starts_with('@') {
        Err("cURL file/stdin uploads need an explicit file selection and are not imported".into())
    } else {
        Ok(v)
    }
}
pub fn parse_curl(input: &str) -> Result<ApiRequest, String> {
    let args = words(input)?;
    if args
        .first()
        .is_none_or(|s| !matches!(s.as_str(), "curl" | "curl.exe"))
    {
        return Err("Expected a cURL command".into());
    }
    let mut request = ApiRequest {
        name: "Imported cURL".into(),
        ..ApiRequest::default()
    };
    request.settings.follow_redirects = false;
    let mut i = 1;
    let mut explicit_method = false;
    let mut data: Vec<String> = vec![];
    let mut form = vec![];
    let mut urlencoded = vec![];
    let mut get_data = false;
    let mut json = false;
    let mut binary_file = None;
    while i < args.len() {
        let arg = &args[i];
        let (option, inline) = if arg.starts_with("--") {
            match arg.split_once('=') {
                Some((a, b)) => (a, Some(b.to_owned())),
                None => (arg.as_str(), None),
            }
        } else if arg.starts_with('-')
            && arg.len() > 2
            && arg.is_char_boundary(2)
            && matches!(
                &arg[..2],
                "-X" | "-H" | "-d" | "-u" | "-b" | "-A" | "-e" | "-F"
            )
        {
            (&arg[..2], Some(arg[2..].to_owned()))
        } else {
            (arg.as_str(), None)
        };
        match option {
            "-X" | "--request" => {
                request.method = value(&args, &mut i, inline)?.to_uppercase();
                explicit_method = true;
            }
            "--url" => {
                if !request.url.is_empty() {
                    return Err("Multiple cURL URLs are unsupported".into());
                }
                request.url = value(&args, &mut i, inline)?;
            }
            "-H" | "--header" => {
                let v = value(&args, &mut i, inline)?;
                let (k, v) = v.split_once(':').ok_or("Header must use Name: value")?;
                request
                    .headers
                    .push(KeyValue::new(k.trim(), v.trim_start()));
            }
            "--data-binary" => {
                let v = value(&args, &mut i, inline)?;
                if let Some(path) = v.strip_prefix('@') {
                    if path == "-" || path.is_empty() {
                        return Err("cURL stdin upload is unsupported".into());
                    }
                    binary_file = Some(std::path::PathBuf::from(path));
                } else {
                    data.push(v);
                }
            }
            "-d" | "--data" | "--data-ascii" => {
                data.push(literal_data(value(&args, &mut i, inline)?)?)
            }
            "--data-raw" => data.push(value(&args, &mut i, inline)?),
            "--json" => {
                data.push(literal_data(value(&args, &mut i, inline)?)?);
                json = true;
            }
            "--data-urlencode" => {
                let v = literal_data(value(&args, &mut i, inline)?)?;
                if v.contains('@') && !v.contains('=') {
                    return Err("cURL file upload is unsupported".into());
                }
                let (k, v) = v.split_once('=').ok_or(
                    "cURL --data-urlencode without a field name is unsupported; use name=value",
                )?;
                urlencoded.push(KeyValue::new(k, v));
            }
            "-F" | "--form" | "--form-string" => {
                let v = value(&args, &mut i, inline)?;
                let (k, v) = v.split_once('=').ok_or("Form field requires name=value")?;
                if option != "--form-string" && v.starts_with('<') {
                    return Err("cURL form file-content mode is unsupported".into());
                }
                let mut row = KeyValue::new(k, v);
                if option != "--form-string"
                    && let Some(path) = v.strip_prefix('@')
                {
                    if path.is_empty() || path == "-" || path.contains(';') {
                        return Err(
                            "Multipart file requires a single path without stdin or modifiers"
                                .into(),
                        );
                    }
                    row.file_path = Some(std::path::PathBuf::from(path));
                    row.value.clear();
                }
                form.push(row);
            }
            "-u" | "--user" => {
                let v = value(&args, &mut i, inline)?;
                let (u, p) = v.split_once(':').unwrap_or((&v, ""));
                request.auth = Auth::Basic {
                    username: u.into(),
                    password: p.into(),
                };
            }
            "--oauth2-bearer" => {
                request.auth = Auth::Bearer {
                    token: value(&args, &mut i, inline)?,
                }
            }
            "-b" | "--cookie" => {
                let v = value(&args, &mut i, inline)?;
                if !v.contains('=') {
                    return Err("cURL cookie files are not imported".into());
                }
                request.headers.push(KeyValue::new("Cookie", v));
            }
            "-A" | "--user-agent" => request
                .headers
                .push(KeyValue::new("User-Agent", value(&args, &mut i, inline)?)),
            "-e" | "--referer" => request
                .headers
                .push(KeyValue::new("Referer", value(&args, &mut i, inline)?)),
            "-I" | "--head" => {
                request.method = "HEAD".into();
                explicit_method = true;
            }
            "-G" | "--get" => get_data = true,
            "-L" | "--location" => request.settings.follow_redirects = true,
            "-k" | "--insecure" => request.settings.verify_tls = false,
            "--max-time" => {
                let seconds = value(&args, &mut i, inline)?
                    .parse::<f64>()
                    .map_err(|_| "Invalid cURL timeout")?;
                if !seconds.is_finite() || seconds <= 0.0 {
                    return Err("Invalid cURL timeout".into());
                }
                request.settings.timeout_ms = (seconds * 1000.0).clamp(100.0, 300000.0) as u64;
            }
            "--compressed" | "--silent" | "-s" | "--show-error" | "-S" | "--include" | "-i"
            | "--globoff" | "-g" => {}
            "--" => {
                i += 1;
                if i + 1 != args.len() || !request.url.is_empty() {
                    return Err("Expected one URL after --".into());
                }
                request.url = args[i].clone();
            }
            s if s.starts_with('-') => return Err(format!("Unsupported cURL option: {s}")),
            _ => {
                if !request.url.is_empty() {
                    return Err("Multiple cURL URLs are unsupported".into());
                }
                request.url = arg.clone();
            }
        }
        i += 1;
    }
    if request.url.is_empty() {
        return Err("cURL URL is missing".into());
    }
    if !request.url.starts_with("http://")
        && !request.url.starts_with("https://")
        && !request.url.starts_with("{{")
    {
        return Err("Only HTTP and HTTPS URLs are supported".into());
    }
    request.set_url(&request.url.clone());
    if binary_file.is_some() && (!data.is_empty() || !urlencoded.is_empty() || !form.is_empty()) {
        return Err("Mixed cURL body modes are unsupported".into());
    }
    if (!form.is_empty() && (!data.is_empty() || !urlencoded.is_empty()))
        || (!data.is_empty() && !urlencoded.is_empty())
    {
        return Err("Mixed cURL body modes are unsupported".into());
    }
    if let Some(path) = binary_file {
        request.body = RequestBody::BinaryFile { path };
        if !explicit_method {
            request.method = "POST".into();
        }
    } else if get_data {
        request.params.extend(urlencoded);
        for d in data {
            request.params.extend(
                url::form_urlencoded::parse(d.as_bytes()).map(|(k, v)| KeyValue::new(k, v)),
            );
        }
        if !explicit_method {
            request.method = "GET".into();
        }
    } else if !form.is_empty() {
        request.body = RequestBody::FormData(form);
        if !explicit_method {
            request.method = "POST".into();
        }
    } else if !urlencoded.is_empty() {
        request.body = RequestBody::UrlEncoded(urlencoded);
        if !explicit_method {
            request.method = "POST".into();
        }
    } else if !data.is_empty() {
        let content_type = if json {
            "application/json"
        } else {
            request
                .headers
                .iter()
                .find(|h| h.key.eq_ignore_ascii_case("Content-Type"))
                .map(|h| h.value.as_str())
                .unwrap_or("application/x-www-form-urlencoded")
        }
        .to_owned();
        request.body = RequestBody::Raw {
            text: data.join("&"),
            content_type,
        };
        if !explicit_method {
            request.method = "POST".into();
        }
        if json
            && !request
                .headers
                .iter()
                .any(|h| h.key.eq_ignore_ascii_case("Accept"))
        {
            request
                .headers
                .push(KeyValue::new("Accept", "application/json"));
        }
    }
    Ok(request)
}
fn quote(v: &str) -> String {
    format!("'{}'", v.replace('\'', "'\\''"))
}
pub fn export_curl(request: &ApiRequest, variables: &Variables) -> Result<String, String> {
    let (url, headers) = resolve_request(request, variables)?;
    let mut out = format!(
        "curl --request {} --url {}",
        quote(&request.method),
        quote(url.as_str())
    );
    for h in &headers {
        out.push_str(&format!(
            " \\\n  --header {}",
            quote(&format!("{}: {}", h.key, h.value))
        ));
    }
    if !request.settings.verify_tls {
        out.push_str(" \\\n  --insecure");
    }
    if request.settings.follow_redirects {
        out.push_str(" \\\n  --location");
    }
    match &request.body {
        RequestBody::None => {}
        RequestBody::Raw { text, content_type } => {
            if !content_type.is_empty()
                && !headers
                    .iter()
                    .any(|h| h.key.eq_ignore_ascii_case("Content-Type"))
            {
                out.push_str(&format!(
                    " \\\n  --header {}",
                    quote(&format!("Content-Type: {content_type}"))
                ));
            }
            let body = interpolate(text, variables)?;
            out.push_str(&format!(" \\\n  --data-raw {}", quote(&body)));
        }
        RequestBody::UrlEncoded(rows) | RequestBody::FormData(rows) => {
            for row in rows.iter().filter(|r| r.enabled && !r.key.is_empty()) {
                out.push_str(&format!(
                    " \\\n  {} {}",
                    if matches!(request.body, RequestBody::FormData(_)) {
                        if row.file_path.is_some() {
                            "--form"
                        } else {
                            "--form-string"
                        }
                    } else {
                        "--data-urlencode"
                    },
                    quote(&format!(
                        "{}={}",
                        interpolate(&row.key, variables)?,
                        if let Some(path) = &row.file_path {
                            format!("@{}", path.to_string_lossy())
                        } else {
                            interpolate(&row.value, variables)?
                        }
                    ))
                ));
            }
        }
        RequestBody::BinaryFile { path } => out.push_str(&format!(
            " \\\n  --data-binary {}",
            quote(&format!("@{}", path.to_string_lossy()))
        )),
        RequestBody::Binary(_) => {
            return Err("Binary body cannot be copied as a text cURL command".into());
        }
    }
    Ok(out)
}

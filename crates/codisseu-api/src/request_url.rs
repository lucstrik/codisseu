use crate::{ApiRequest, KeyValue, Variables, interpolate};
fn query_parts(text: &str) -> (&str, Option<&str>, Option<&str>) {
    let (head, fragment) = text
        .split_once('#')
        .map_or((text, None), |(a, b)| (a, Some(b)));
    let (base, query) = head
        .split_once('?')
        .map_or((head, None), |(a, b)| (a, Some(b)));
    (base, query, fragment)
}
fn encode_query(text: &str) -> String {
    // Keep template notation visible; interpolation still happens on decoded rows at Send.
    url::form_urlencoded::byte_serialize(text.as_bytes())
        .collect::<String>()
        .replace("%7B%7B", "{{")
        .replace("%7D%7D", "}}")
}
impl ApiRequest {
    pub fn display_url(&self) -> String {
        let (base, existing, fragment) = query_parts(&self.url);
        let mut parts = vec![];
        if let Some(q) = existing.filter(|q| !q.is_empty()) {
            parts.push(q.to_owned());
        }
        parts.extend(
            self.params
                .iter()
                .filter(|p| p.enabled && !p.key.is_empty())
                .map(|p| format!("{}={}", encode_query(&p.key), encode_query(&p.value))),
        );
        let mut out = base.to_owned();
        if !parts.is_empty() {
            out.push('?');
            out.push_str(&parts.join("&"));
        }
        if let Some(fragment) = fragment {
            out.push('#');
            out.push_str(fragment);
        }
        out
    }
    /// URL edits replace enabled query rows; disabled rows and matching metadata survive.
    pub fn set_url(&mut self, text: &str) {
        let (base, query, fragment) = query_parts(text);
        let mut old = self.params.clone();
        let mut params = vec![];
        if let Some(query) = query {
            for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
                let mut row = if let Some(index) = old
                    .iter()
                    .position(|r| r.enabled && r.key == k && r.value == v)
                {
                    old.remove(index)
                } else {
                    KeyValue::new(k.as_ref(), v.as_ref())
                };
                row.enabled = true;
                params.push(row);
            }
        }
        params.extend(old.into_iter().filter(|p| !p.enabled));
        self.params = params;
        self.url = base.to_owned();
        if let Some(fragment) = fragment {
            self.url.push('#');
            self.url.push_str(fragment);
        }
        let names = base
            .split('/')
            .filter_map(|s| s.strip_prefix(':'))
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        self.path_variables.retain(|p| names.contains(&p.key));
        for name in names {
            if !self.path_variables.iter().any(|p| p.key == name) {
                self.path_variables.push(KeyValue::new(name, ""));
            }
        }
    }
    pub(crate) fn resolved_url_text(&self, variables: &Variables) -> Result<String, String> {
        let mut text = String::new();
        for (index, part) in self.url.split('/').enumerate() {
            if index > 0 {
                text.push('/');
            }
            // Path variables are whole segments, never authority/port or fragments.
            let (segment, suffix) = part
                .find(['?', '#'])
                .map_or((part, ""), |n| (&part[..n], &part[n..]));
            if let Some(row) = self
                .path_variables
                .iter()
                .find(|p| p.enabled && segment == format!(":{}", p.key))
            {
                let value = interpolate(&row.value, variables)?;
                for byte in value.bytes() {
                    if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                        text.push(char::from(byte));
                    } else {
                        text.push_str(&format!("%{byte:02X}"));
                    }
                }
                text.push_str(suffix);
            } else {
                text.push_str(part);
            }
        }
        interpolate(&text, variables)
    }
}

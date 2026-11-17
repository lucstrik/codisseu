//! A read-only origin for the unmodified SDK DevTools frontend. No host/IDE APIs.
use cef::*;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const BOOTSTRAP: &[u8] = include_bytes!("../assets/bootstrap.js");
pub const ORIGIN: &str = "codisseu-devtools://frontend";
pub type Registry = Arc<Mutex<BTreeMap<i32, String>>>;

pub fn register(registrar: &mut SchemeRegistrar) {
    let options = SchemeOptions::STANDARD.get_raw()
        | SchemeOptions::SECURE.get_raw()
        | SchemeOptions::DISPLAY_ISOLATED.get_raw()
        | SchemeOptions::CORS_ENABLED.get_raw()
        | SchemeOptions::FETCH_ENABLED.get_raw();
    registrar.add_custom_scheme(Some(&"codisseu-devtools".into()), options as i32);
}

pub fn install(runtime: &Path, registry: Registry) -> Result<(), String> {
    let root = runtime
        .join("devtools-assets")
        .canonicalize()
        .map_err(|_| "Prepare official DevTools assets first")?;
    let body =
        std::fs::read(root.join("manifest.json")).map_err(|_| "Missing DevTools inventory")?;
    if body.len() > 2 * 1024 * 1024 {
        return Err("DevTools inventory exceeds limit".into());
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| "Invalid DevTools inventory")?;
    verify_embedding(&root, &manifest)?;
    let resources = manifest["resources"]
        .as_object()
        .ok_or("Invalid DevTools inventory")?
        .keys()
        .cloned()
        .chain(std::iter::once("CODISSEU_BOOTSTRAP_JS".into()))
        .collect::<std::collections::BTreeSet<_>>();
    let mut factory = Assets::new(root, Arc::new(resources), registry);
    if register_scheme_handler_factory(
        Some(&"codisseu-devtools".into()),
        Some(&"frontend".into()),
        Some(&mut factory),
    ) != 1
    {
        return Err("Unable to register the confined DevTools origin".into());
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn verify_embedding(root: &Path, manifest: &serde_json::Value) -> Result<(), String> {
    let bytes = std::fs::read(root.join("embedding.json"))
        .map_err(|_| "Missing DevTools embedding provenance")?;
    if bytes.len() > 16384 {
        return Err("DevTools embedding provenance exceeds limit".into());
    }
    let adapter: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid DevTools embedding provenance")?;
    if adapter["adapter_version"] != 4
        || adapter["cef_version"] != "156.0.15+gcf671c0+chromium-156.0.8078.80"
        || adapter["sdk_javascript_modified"] != false
        || adapter["sdk_css_modified"] != false
    {
        return Err("Unsupported DevTools embedding adapter".into());
    }
    let boot = std::fs::read(root.join("CODISSEU_BOOTSTRAP_JS.bin"))
        .map_err(|_| "Missing own DevTools embedding adapter")?;
    if boot != BOOTSTRAP || adapter["adapter_js_sha256"].as_str() != Some(digest(&boot).as_str()) {
        return Err("DevTools embedding loader changed".into());
    }
    let mut document = None;
    for (name, field) in [
        ("DEVTOOLS_APP_HTML", "original_html_sha256"),
        (
            "ENTRYPOINTS_INSPECTOR_MAIN_INSPECTOR_MAIN_JS",
            "connector_js_sha256",
        ),
        (
            "ENTRYPOINTS_DEVTOOLS_APP_DEVTOOLS_APP_JS",
            "entrypoint_js_sha256",
        ),
    ] {
        let path = root
            .join(format!("{name}.bin"))
            .canonicalize()
            .map_err(|_| "Missing official DevTools bootstrap")?;
        if !path.starts_with(root) {
            return Err("DevTools bootstrap path escaped private resources".into());
        }
        let body = std::fs::read(path).map_err(|_| "Unreadable official DevTools bootstrap")?;
        if body.len() > 4 * 1024 * 1024 {
            return Err("DevTools bootstrap exceeds limit".into());
        }
        let hash = digest(&body);
        if adapter[field].as_str() != Some(hash.as_str())
            || manifest["resources"][name]["sha256"].as_str() != Some(hash.as_str())
        {
            return Err("Official DevTools bootstrap changed; review the adapter".into());
        }
        if name == "ENTRYPOINTS_INSPECTOR_MAIN_INSPECTOR_MAIN_JS"
            && (!body
                .windows(b"registerEarlyInitializationRunnable".len())
                .any(|w| w == b"registerEarlyInitializationRunnable")
                || !body
                    .windows(b"initMainConnection".len())
                    .any(|w| w == b"initMainConnection"))
        {
            return Err("DevTools page connector API changed".into());
        }
        if name == "ENTRYPOINTS_DEVTOOLS_APP_DEVTOOLS_APP_JS"
            && body
                .windows(b"../inspector_main/inspector_main.js".len())
                .any(|w| w == b"../inspector_main/inspector_main.js")
        {
            return Err("DevTools app already initializes its connector; review adapter".into());
        }
        if name == "DEVTOOLS_APP_HTML" {
            document = Some(body);
        }
    }
    let document = document.ok_or("Missing DevTools document")?;
    let text = std::str::from_utf8(&document).map_err(|_| "Invalid DevTools document")?;
    if text
        .matches(
            "<script type=\"module\" src=\"./entrypoints/devtools_app/devtools_app.js\"></script>",
        )
        .count()
        != 1
    {
        return Err("DevTools document bootstrap changed".into());
    }
    let served = standalone_document("DEVTOOLS_APP_HTML", document);
    if adapter["served_html_sha256"].as_str() != Some(digest(&served).as_str()) {
        return Err("DevTools embedding response differs from its provenance".into());
    }
    Ok(())
}

fn resource(url: &str) -> Option<(String, &'static str)> {
    let url = url::Url::parse(url).ok()?;
    if url.scheme() != "codisseu-devtools"
        || url.host_str() != Some("frontend")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let path = url.path().strip_prefix("/bundled/")?;
    if path.is_empty()
        || path.len() > 512
        || !path
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"/_.-".contains(&c))
    {
        return None;
    }
    let mut name = path
        .bytes()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase() as char
            } else {
                '_'
            }
        })
        .collect::<String>();
    if name == "PROTOCOL_JSON" {
        name = "COMPRESSED_PROTOCOL_JSON".into();
    }
    let mime = match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("html") => "text/html",
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        _ => "application/octet-stream",
    };
    Some((name, mime))
}

fn standalone_document(name: &str, body: Vec<u8>) -> Vec<u8> {
    if name != "DEVTOOLS_APP_HTML" {
        return body;
    }
    // The hosted adapter must initialize the SDK's own page connector before the
    // standard app runs its early initialization registry. All SDK JavaScript is
    // served byte-for-byte unchanged; only this embedding bootstrap is inserted.
    let Ok(html) = String::from_utf8(body) else {
        return Vec::new();
    };
    html.replacen(
        "<script type=\"module\" src=\"./entrypoints/devtools_app/devtools_app.js\"></script>",
        "<script type=\"module\" src=\"./codisseu_bootstrap.js\"></script>",
        1,
    )
    .into_bytes()
}

wrap_scheme_handler_factory! {struct Assets{root:PathBuf,resources:Arc<std::collections::BTreeSet<String>>,registry:Registry}impl SchemeHandlerFactory{
 fn create(&self,browser:Option<&mut Browser>,_frame:Option<&mut Frame>,_scheme:Option<&CefString>,request:Option<&mut Request>)->Option<ResourceHandler>{
  let endpoint=browser.and_then(|b|self.registry.lock().unwrap().get(&b.identifier()).cloned());
  let url=request.as_ref().map(|r|CefString::from(&r.url()).to_string()).unwrap_or_default();
  let resource=resource(&url).filter(|(name,_)|self.resources.contains(name));
  let mut status=404;let mut mime="text/plain";let mut bytes=Vec::new();
  if let(Some(endpoint),Some((name,kind)))=(endpoint.as_ref(),resource){
   let file=self.root.join(format!("{name}.bin"));
   if let Ok(path)=file.canonicalize()&&path.starts_with(&self.root)&&let Ok(body)=std::fs::read(&path)&&body.len()<=16*1024*1024{bytes=standalone_document(&name,body);mime=kind;status=200;}
   let _=endpoint;
  }
  if status!=200{let path=url::Url::parse(&url).ok().map(|url|url.path().to_owned()).unwrap_or_default();eprintln!("DevTools asset unavailable: {}",crate::frontend_diagnostic(&path));}
  Some(Asset::new(Arc::new(bytes),Arc::new(Mutex::new(0)),mime.into(),status,endpoint.unwrap_or_default()))
 }
}}
wrap_resource_handler! {struct Asset{bytes:Arc<Vec<u8>>,offset:Arc<Mutex<usize>>,mime:String,status:i32,endpoint:String}impl ResourceHandler{
 fn open(&self,_request:Option<&mut Request>,handle:Option<&mut i32>,_callback:Option<&mut Callback>)->i32{if let Some(handle)=handle{*handle=1;}1}
 fn response_headers(&self,response:Option<&mut Response>,length:Option<&mut i64>,_redirect:Option<&mut CefString>){
  if let Some(length)=length{*length=self.bytes.len()as i64;}
  if let Some(response)=response{response.set_status(self.status);response.set_mime_type(Some(&self.mime.as_str().into()));
   let csp=format!("default-src 'self' data: blob:; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self' {}; worker-src 'self' blob:; frame-src 'self' data: blob:; object-src 'none'; base-uri 'self'; form-action 'none'",self.endpoint);
   response.set_header_by_name(Some(&"Content-Security-Policy".into()),Some(&csp.as_str().into()),1);
   response.set_header_by_name(Some(&"X-Content-Type-Options".into()),Some(&"nosniff".into()),1);
   response.set_header_by_name(Some(&"Cache-Control".into()),Some(&"no-store".into()),1);
  }
 }
 fn read(&self,output:*mut u8,requested:i32,read:Option<&mut i32>,_callback:Option<&mut ResourceReadCallback>)->i32{
  let Some(read)=read else{return 0};*read=0;if requested<=0||output.is_null(){return 0;}
  let mut offset=self.offset.lock().unwrap();let amount=(requested as usize).min(self.bytes.len().saturating_sub(*offset));if amount==0{return 0;}
  // SAFETY: CEF supplies a writable requested-byte buffer; our bounded slice
  // remains owned by this resource throughout the synchronous copy.
  unsafe{std::ptr::copy_nonoverlapping(self.bytes[*offset..].as_ptr(),output,amount);}
  *offset+=amount;*read=amount as i32;1
 }
}}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn asset_lookup_cannot_escape_the_read_only_sdk_inventory() {
        assert_eq!(
            resource("codisseu-devtools://frontend/bundled/core/sdk/sdk.js").unwrap(),
            ("CORE_SDK_SDK_JS".into(), "text/javascript")
        );
        for url in [
            "file:///etc/passwd",
            "codisseu-devtools://other/bundled/a.js",
            "codisseu-devtools://frontend/bundled/../../etc/passwd",
            "codisseu-devtools://frontend/bundled/%2fetc%2fpasswd",
        ] {
            assert!(resource(url).is_none());
        }
    }
}

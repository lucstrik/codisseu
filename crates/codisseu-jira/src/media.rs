//! Opt-in images from Jira metadata. Auth is restricted to the configured origins.
use crate::{Connection, Secret};
use reqwest::Client;
use std::{
    collections::{HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::Duration,
};
pub struct Raster {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}
pub struct ImageClient {
    http: Client,
    connection: Connection,
    token: Secret,
}
impl ImageClient {
    pub(crate) fn new(connection: Connection, token: Secret) -> Result<Self, String> {
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(6))
            .connect_timeout(Duration::from_secs(3))
            .build()
            .map_err(|_| "Imagens indisponíveis")?;
        Ok(Self {
            http,
            connection,
            token,
        })
    }
    fn trusted(&self, url: &url::Url) -> bool {
        if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
            return false;
        }
        if self.authenticated(url) {
            return true;
        }
        let host = url.host_str().unwrap_or_default();
        if matches!(host, "i0.wp.com" | "i1.wp.com" | "i2.wp.com" | "i3.wp.com") {
            return true;
        }
        [
            "atlassian.net",
            "atlassian.com",
            "atl-paas.net",
            "gravatar.com",
        ]
        .iter()
        .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
    }
    fn authenticated(&self, u: &url::Url) -> bool {
        [&self.connection.site, &self.connection.api]
            .iter()
            .any(|s| {
                url::Url::parse(s).is_ok_and(|a| {
                    a.origin() == u.origin()
                        && (a.host_str() != Some("api.atlassian.com")
                            || u.path()
                                .starts_with(&format!("{}/", a.path().trim_end_matches('/'))))
                })
            })
    }
    pub(crate) async fn fetch(&self, value: &str) -> Result<Raster, String> {
        let mut u = url::Url::parse(value).map_err(|_| "URL de imagem inválida")?;
        for _ in 0..4 {
            if !self.trusted(&u) {
                return Err("Imagem fora das origens Jira/Atlassian permitidas".into());
            }
            let mut req = self.http.get(u.clone()).header("Accept", "*/*");
            if self.authenticated(&u) {
                req = req.basic_auth(&self.connection.email, Some(self.token.expose()))
            }
            let mut r = req
                .send()
                .await
                .map_err(|_| "Não foi possível carregar a imagem")?;
            if r.status().is_redirection() {
                let target = r
                    .headers()
                    .get("location")
                    .and_then(|h| h.to_str().ok())
                    .ok_or("Imagem redirecionada sem destino")?;
                u = u.join(target).map_err(|_| "Destino de imagem inválido")?;
                continue;
            }
            if !r.status().is_success() {
                return Err("Imagem indisponível ou sem permissão".into());
            }
            const MAX: usize = 4 * 1024 * 1024;
            if r.content_length().is_some_and(|l| l > MAX as u64) {
                return Err("Imagem acima de4MiB".into());
            }
            let mut bytes = vec![];
            while let Some(b) = r.chunk().await.map_err(|_| "Imagem interrompida")? {
                if bytes.len() + b.len() > MAX {
                    return Err("Imagem acima de4MiB".into());
                }
                bytes.extend_from_slice(&b)
            }
            return decode(&bytes);
        }
        Err("Muitos redirecionamentos de imagem".into())
    }
}
pub(crate) fn decode(bytes: &[u8]) -> Result<Raster, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "Formato de imagem inválido")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let img = reader
        .decode()
        .map_err(|_| "Formato de imagem indisponível, inválido ou acima do limite")?;
    if u64::from(img.width()) * u64::from(img.height()) > 4_000_000 {
        return Err("Imagem acima de4milhões de pixels".into());
    }
    let img = if img.width() > 1600 || img.height() > 1200 {
        img.thumbnail(1600, 1200)
    } else {
        img
    };
    let img = img.to_rgba8();
    Ok(Raster {
        width: img.width() as usize,
        height: img.height() as usize,
        rgba: img.into_raw(),
    })
}
struct Job {
    epoch: u64,
    client: Arc<ImageClient>,
    url: String,
}
type MediaReply = (u64, String, Result<Raster, String>);
pub struct Media {
    client: Option<Arc<ImageClient>>,
    tx: Option<SyncSender<Job>>,
    rx: Option<Receiver<MediaReply>>,
    epoch: Arc<AtomicU64>,
    pending: HashSet<String>,
    pub completed: VecDeque<(String, Result<Raster, String>)>,
}
impl Default for Media {
    fn default() -> Self {
        Self {
            client: None,
            tx: None,
            rx: None,
            epoch: Arc::new(AtomicU64::new(0)),
            pending: HashSet::new(),
            completed: VecDeque::new(),
        }
    }
}
impl Media {
    pub(crate) fn session(&mut self, c: Option<ImageClient>) {
        self.client = c.map(Arc::new);
        self.epoch.fetch_add(1, Ordering::Relaxed);
        self.pending.clear();
        self.completed.clear();
    }
    pub fn request(&mut self, url: &str) -> bool {
        if self.pending.contains(url) || self.pending.len() >= 8 || url.len() > 4096 {
            return false;
        }
        let Some(c) = self.client.clone() else {
            return false;
        };
        if self.tx.is_none() {
            let (tx, jobs) = mpsc::sync_channel::<Job>(8);
            let (replies, rx) = mpsc::sync_channel(8);
            let epoch = self.epoch.clone();
            if std::thread::Builder::new()
                .name("codisseu-jira-images".into())
                .spawn(move || {
                    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    else {
                        return;
                    };
                    while let Ok(j) = jobs.recv() {
                        if j.epoch != epoch.load(Ordering::Relaxed) {
                            continue;
                        }
                        let r = rt.block_on(j.client.fetch(&j.url));
                        if j.epoch == epoch.load(Ordering::Relaxed)
                            && replies.send((j.epoch, j.url, r)).is_err()
                        {
                            break;
                        }
                    }
                })
                .is_err()
            {
                return false;
            }
            self.tx = Some(tx);
            self.rx = Some(rx);
        }
        if self
            .tx
            .as_ref()
            .unwrap()
            .try_send(Job {
                epoch: self.epoch.load(Ordering::Relaxed),
                client: c,
                url: url.into(),
            })
            .is_ok()
        {
            self.pending.insert(url.into());
            true
        } else {
            false
        }
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.rx {
            while let Ok((epoch, url, r)) = rx.try_recv() {
                if epoch == self.epoch.load(Ordering::Relaxed) {
                    self.pending.remove(&url);
                    if self.completed.len() >= 8 {
                        self.completed.pop_front();
                    }
                    self.completed.push_back((url, r));
                    changed = true
                }
            }
        }
        changed
    }
    pub fn pending(&self) -> bool {
        !self.pending.is_empty()
    }
}
impl Drop for Media {
    fn drop(&mut self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_origins_confine_auth_and_decoder_is_bounded() {
        let c = ImageClient::new(
            Connection {
                site: "https://fixture.atlassian.net".into(),
                api: "https://api.atlassian.com/ex/jira/fixture".into(),
                email: "me@example.invalid".into(),
            },
            Secret::new("canary".into()),
        )
        .unwrap();
        let url = |s: &str| url::Url::parse(s).unwrap();
        assert!(c.authenticated(&url("https://fixture.atlassian.net/secure/attachment/1")));
        assert!(!c.authenticated(&url(
            "https://avatar-management--avatars.us-west-2.prod.public.atl-paas.net/avatar/1"
        )));
        assert!(c.trusted(&url(
            "https://avatar-management--avatars.us-west-2.prod.public.atl-paas.net/avatar/1"
        )));
        assert!(!c.trusted(&url("https://atlassian.net.attacker.invalid/a")));
        assert!(!c.trusted(&url("http://fixture.atlassian.net/a")));
        assert!(!c.trusted(&url("https://127.0.0.1/a")));
        assert!(!c.authenticated(&url("https://api.atlassian.com/ex/jira/other/a")));
        let img = image::RgbaImage::from_pixel(48, 48, image::Rgba([80, 120, 180, 255]));
        let mut buf = std::io::Cursor::new(vec![]);
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let r = decode(buf.get_ref()).unwrap();
        assert_eq!((r.width, r.height, r.rgba.len()), (48, 48, 48 * 48 * 4));
        assert!(decode(b"<svg onload='alert(1)'/>").is_err());
    }
}

use crate::MockDefinition;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub struct MockServer {
    pub url: String,
    cancel: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}
impl MockServer {
    /// Called on a background worker by ApiService. Listens only on loopback.
    pub fn start(definition: MockDefinition) -> Result<Self, String> {
        if definition.routes.len() > 1024
            || definition.routes.iter().any(|r| {
                r.body.len() > 4 * 1024 * 1024
                    || r.headers.len() > 128
                    || !(100..=599).contains(&r.status)
                    || r.headers.iter().any(|h| {
                        h.key.contains(['\r', '\n', ':']) || h.value.contains(['\r', '\n'])
                    })
            })
        {
            return Err("Mock routes exceed limits or contain invalid headers/status".into());
        }
        let listener =
            TcpListener::bind("127.0.0.1:0").map_err(|_| "Could not start loopback mock server")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "Could not configure mock server")?;
        let url = format!(
            "http://{}",
            listener
                .local_addr()
                .map_err(|_| "Could not read mock address")?
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let join=thread::Builder::new().name("codisseu-api-mock".into()).spawn(move||{while !stop.load(Ordering::Acquire){match listener.accept(){Ok((mut socket,_))=>{
            let _=socket.set_read_timeout(Some(Duration::from_millis(100)));let _=socket.set_write_timeout(Some(Duration::from_millis(100)));let started=Instant::now();let mut request=vec![];let mut buffer=[0u8;2048];
            while request.len()<64*1024&&!stop.load(Ordering::Acquire)&&started.elapsed()<Duration::from_secs(2){match socket.read(&mut buffer){Ok(0)=>break,Ok(n)=>{request.extend_from_slice(&buffer[..n]);if request.windows(4).any(|w|w==b"\r\n\r\n"){break;}},Err(e)if matches!(e.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::TimedOut)=>{},Err(_)=>break}}
            if stop.load(Ordering::Acquire){break;}
            let head=String::from_utf8_lossy(&request);let mut line=head.lines().next().unwrap_or("").split_whitespace();let method=line.next().unwrap_or("");let path=line.next().unwrap_or("").split('?').next().unwrap_or("");let route=definition.routes.iter().find(|r|r.method.eq_ignore_ascii_case(method)&&r.path==path);
            let(status,body)=route.map(|r|(r.status,r.body.as_str())).unwrap_or((404,"{\"error\":\"No mock route\"}"));let mut response=format!("HTTP/1.1 {status} Mock\r\nContent-Length: {}\r\nConnection: close\r\n",body.len());
            if let Some(route)=route{for h in route.headers.iter().filter(|h|h.enabled&&!matches!(h.key.to_ascii_lowercase().as_str(),"content-length"|"connection"|"transfer-encoding")){response.push_str(&format!("{}: {}\r\n",h.key,h.value));}}response.push_str("\r\n");let _=socket.write_all(response.as_bytes());if method!="HEAD"{let _=socket.write_all(body.as_bytes());}
        },Err(e)if e.kind()==std::io::ErrorKind::WouldBlock=>thread::sleep(Duration::from_millis(10)),Err(_)=>break}}}).map_err(|_|"Could not start mock thread")?;
        Ok(Self {
            url,
            cancel,
            join: Some(join),
        })
    }
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn is_finished(&self) -> bool {
        self.join.as_ref().is_none_or(|j| j.is_finished())
    }
}
impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop();
        if self.is_finished()
            && let Some(join) = self.join.take()
        {
            let _ = join.join();
        }
    }
}

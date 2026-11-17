//! Controlled contract peers prove routing/auth bounds, not browser debugging.
use super::*;
use crate::{
    DebugCommand, DebugConfiguration, DebugOutput, DebugRequest, DebugRuntime, DebugService,
};
use std::{io::Read, sync::mpsc};
use tungstenite::client::IntoClientRequest;

fn info() -> Value {
    json!({"targetId":"owned-contract-target","type":"page","url":"http://127.0.0.1:3000/","title":"Contract peer"})
}
struct ContractPeer {
    receive: mpsc::Receiver<String>,
    send: mpsc::SyncSender<String>,
}
impl Peer for ContractPeer {
    fn send(&self, raw: String) -> Result<(), String> {
        let value: Value = serde_json::from_str(&raw).unwrap();
        if value["id"] == INFO_ID {
            self.send
                .try_send(json!({"id":INFO_ID,"result":{"targetInfo":info()}}).to_string())
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    fn receive(&self, timeout: Duration) -> Result<Option<String>, String> {
        match self.receive.recv_timeout(timeout) {
            Ok(s) => Ok(Some(s)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err("contract peer closed".into()),
        }
    }
}
fn bridge() -> BrowserDebugBridge {
    bridge_mode(GatewayMode::Adapter)
}
fn bridge_mode(mode: GatewayMode) -> BrowserDebugBridge {
    let (send, receive) = mpsc::sync_channel(8);
    BrowserDebugBridge::start_peer_mode(Box::new(ContractPeer { send, receive }), 1, 9, mode)
        .unwrap()
}

#[test]
fn browser_frontend_page_mode_forwards_dom_without_synthetic_sessions_and_accepts_only_devtools_origin()
 {
    let mut router = Router::new_mode(info(), 9, GatewayMode::Page);
    let request = json!({"id":1,"method":"DOM.getDocument","params":{"depth":1}});
    let Routed::Forward(forwarded) = router.request(request.clone()).unwrap() else {
        panic!("DOM commands require the actual engine")
    };
    assert_eq!(forwarded, request);
    let actual = json!({"id":1,"result":{"root":{"nodeId":137}}});
    assert_eq!(router.event(actual.clone()).unwrap(), Some(actual));
    for method in [
        "Browser.close",
        "Target.getTargets",
        "Target.closeTarget",
        "Target.createTarget",
    ] {
        assert!(
            local(
                router
                    .request(json!({"id":2,"method":method,"params":{}}))
                    .unwrap()
            )[0]
            .get("error")
            .is_some()
        );
    }
    for origin in ["devtools://devtools", "codisseu-devtools://frontend"] {
        let mut bridge = bridge_mode(GatewayMode::Page);
        let endpoint = bridge.endpoint();
        let url = bridge.frontend_url().unwrap();
        assert!(url.as_str().starts_with(
            "devtools://devtools/bundled/devtools_app.html?can_dock=false&ws=127.0.0.1:"
        ));
        let token = endpoint.inspect_uri.as_str().rsplit('/').next().unwrap();
        assert!(!format!("{url:?}").contains(token));
        let address = ("127.0.0.1", endpoint.port);
        let mut bad = endpoint.inspect_uri.as_str().into_client_request().unwrap();
        bad.headers_mut()
            .insert("Origin", "http://127.0.0.1".parse().unwrap());
        assert!(tungstenite::client(bad, TcpStream::connect(address).unwrap()).is_err());
        let mut bad = endpoint.inspect_uri.as_str().into_client_request().unwrap();
        bad.headers_mut()
            .insert("Host", "localhost".parse().unwrap());
        assert!(tungstenite::client(bad, TcpStream::connect(address).unwrap()).is_err());
        let mut correct = endpoint.inspect_uri.as_str().into_client_request().unwrap();
        correct
            .headers_mut()
            .insert("Origin", origin.parse().unwrap());
        let (_socket, _) =
            tungstenite::client(correct, TcpStream::connect(address).unwrap()).unwrap();
        assert!(bridge.shutdown(Duration::from_secs(1)));
    }
}
fn local(routed: Routed) -> Vec<Value> {
    match routed {
        Routed::Local(v) => v,
        _ => panic!("expected local routing reply"),
    }
}

#[test]
fn browser_routing_never_invents_runtime_results_or_reaches_another_target() {
    let mut router = Router::new(info(), 9);
    let browser = local(
        router
            .request(json!({"id":1,"method":"Target.attachToBrowserTarget","params":{}}))
            .unwrap(),
    )[0]["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    let wrong=local(router.request(json!({"id":2,"sessionId":browser,"method":"Target.attachToTarget","params":{"targetId":"other-tab"}})).unwrap());
    assert!(wrong[0].get("error").is_some());
    let page=local(router.request(json!({"id":3,"sessionId":browser,"method":"Target.attachToTarget","params":{"targetId":"owned-contract-target"}})).unwrap())[0]["result"]["sessionId"].as_str().unwrap().to_owned();
    let input =
        json!({"id":4,"sessionId":page,"method":"Runtime.evaluate","params":{"expression":"6*7"}});
    let Routed::Forward(forwarded) = router.request(input).unwrap() else {
        panic!("Runtime must be forwarded to the actual engine")
    };
    assert_eq!(
        forwarded,
        json!({"id":4,"method":"Runtime.evaluate","params":{"expression":"6*7"}})
    );
    let engine_value = json!({"id":4,"result":{"result":{"type":"number","value":137}}});
    let output = router.event(engine_value.clone()).unwrap().unwrap();
    assert_eq!(
        output["result"], engine_value["result"],
        "engine data is untouched, including a deliberately non-42 fixture value"
    );
    assert_eq!(output["sessionId"], page);
    for method in ["Browser.close", "Target.closeTarget", "Target.createTarget"] {
        assert!(
            local(
                router
                    .request(json!({"id":5,"sessionId":page,"method":method,"params":{}}))
                    .unwrap()
            )[0]
            .get("error")
            .is_some()
        );
    }
    assert!(local(router.request(json!({"id":6,"sessionId":page,"method":"Page.navigate","params":{"url":"file:///etc/passwd"}})).unwrap())[0].get("error").is_some());
    assert!(local(router.request(json!({"id":7,"sessionId":"unknown-session","method":"Debugger.enable","params":{}})).unwrap())[0].get("error").is_some());
    assert!(parse(&"x".repeat(MAX_CDP + 1)).is_err());
}

#[test]
fn browser_gateway_rejects_origin_wrong_token_host_and_second_client_then_reclaims_port() {
    let mut bridge = bridge();
    let endpoint = bridge.endpoint();
    let address = ("127.0.0.1", endpoint.port);
    let request =
        |uri: &str| -> tungstenite::http::Request<()> { uri.into_client_request().unwrap() };
    let mut hostile = request(endpoint.inspect_uri.as_str());
    hostile
        .headers_mut()
        .insert("Origin", "http://hostile.invalid".parse().unwrap());
    assert!(tungstenite::client(hostile, TcpStream::connect(address).unwrap()).is_err());
    let mut hostile = request(endpoint.inspect_uri.as_str());
    hostile
        .headers_mut()
        .insert("Host", "127.0.0.1:1".parse().unwrap());
    assert!(tungstenite::client(hostile, TcpStream::connect(address).unwrap()).is_err());
    let mut hostile = request(endpoint.inspect_uri.as_str());
    hostile
        .headers_mut()
        .insert("Host", "hostile.invalid".parse().unwrap());
    assert!(tungstenite::client(hostile, TcpStream::connect(address).unwrap()).is_err());
    assert!(
        tungstenite::client(
            format!("ws://127.0.0.1:{}/wrong", endpoint.port),
            TcpStream::connect(address).unwrap()
        )
        .is_err()
    );
    let mut discovery = TcpStream::connect(address).unwrap();
    discovery
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    discovery
        .write_all(
            format!(
                "GET /json/version HTTP/1.1\r\nHost: localhost:{}\r\n\r\n",
                endpoint.port
            )
            .as_bytes(),
        )
        .unwrap();
    let mut body = String::new();
    discovery.read_to_string(&mut body).unwrap();
    assert!(body.contains("200 OK"));
    assert!(body.contains("/unavailable"));
    let token = endpoint.inspect_uri.as_str().rsplit('/').next().unwrap();
    assert_eq!(token.len(), 64);
    assert!(!body.contains(token));
    let mut adapter_request = request(endpoint.inspect_uri.as_str());
    adapter_request
        .headers_mut()
        .insert("Host", "localhost".parse().unwrap());
    let (mut ws, _) =
        tungstenite::client(adapter_request, TcpStream::connect(address).unwrap()).unwrap();
    assert!(
        tungstenite::client(
            endpoint.inspect_uri.as_str(),
            TcpStream::connect(address).unwrap()
        )
        .is_err()
    );
    ws.send(Message::Text(
        json!({"id":1,"method":"Target.attachToBrowserTarget","params":{}})
            .to_string()
            .into(),
    ))
    .unwrap();
    assert!(
        ws.read()
            .unwrap()
            .to_text()
            .unwrap()
            .contains("codisseu-routing-browser")
    );
    assert!(bridge.shutdown(Duration::from_secs(1)));
    assert!(TcpStream::connect(address).is_err());
}

#[test]
fn browser_secrets_are_redacted_skipped_and_restored_configs_cannot_launch_external_chrome() {
    let bridge = bridge();
    let endpoint = bridge.endpoint();
    let raw = endpoint.inspect_uri.as_str();
    let token = raw.rsplit('/').next().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let config = DebugConfiguration {
        runtime: DebugRuntime::Chrome,
        request: DebugRequest::Attach,
        integrated_browser: true,
        browser_tab: Some(1),
        browser_inspect_uri: Some(endpoint.inspect_uri.clone()),
        port: endpoint.port,
        cwd: dir.path().into(),
        trusted: true,
        ..Default::default()
    };
    assert!(!format!("{config:?} {endpoint:?} {bridge:?}").contains(token));
    let encoded = serde_json::to_string(&config).unwrap();
    assert!(!encoded.contains(token));
    assert!(!encoded.contains("browser_tab"));
    assert!(!encoded.contains("browser_inspect_uri"));
    let mut restored: DebugConfiguration = serde_json::from_str(&encoded).unwrap();
    assert!(restored.integrated_browser);
    assert!(restored.browser_inspect_uri.is_none());
    restored.trusted = true;
    let mut service = DebugService::default();
    assert!(
        service
            .execute(DebugCommand::Start(restored))
            .unwrap_err()
            .contains("live integrated browser")
    );
    assert!(service.sessions.is_empty());
    let mut session = crate::DebugSession::new(1, &config);
    session.last_error = Some(format!("could not connect {raw}"));
    session.output.push_back(DebugOutput {
        category: "stderr".into(),
        text: format!("nonce={token}"),
    });
    endpoint.inspect_uri.redact_session(&mut session);
    assert!(!format!("{session:?}").contains(token));
    assert!(session.last_error.unwrap().contains("redacted"));
}

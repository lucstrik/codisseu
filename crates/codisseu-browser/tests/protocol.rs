use codisseu_browser::{Command, Event, KeyKind, Modifiers, Surface, protocol};
use std::io::Cursor;
fn packet(json: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut bytes = (json.len() as u32).to_le_bytes().to_vec();
    bytes.extend((payload.len() as u32).to_le_bytes());
    bytes.extend(json);
    bytes.extend(payload);
    bytes
}
#[test]
fn typed_unicode_commands_frames_cdp_and_composition_roundtrip_without_pointer_or_script_fields() {
    let command = Command::Composition {
        id: 1,
        surface: Surface::Page,
        text: "ação 😀".into(),
        selection_start: 0,
        selection_end: 7,
    };
    let mut bytes = vec![];
    protocol::write_command(&mut bytes, &command).unwrap();
    assert_eq!(
        protocol::read_command(&mut Cursor::new(bytes)).unwrap(),
        command
    );
    let key = Command::Key {
        id: 1,
        surface: Surface::Page,
        kind: KeyKind::Char,
        windows_key_code: 65,
        native_key_code: 38,
        modifiers: Modifiers {
            ctrl: true,
            ..Default::default()
        },
        character: Some('雨'),
        unmodified: Some('雨'),
    };
    let mut bytes = vec![];
    protocol::write_command(&mut bytes, &key).unwrap();
    assert_eq!(
        protocol::read_command(&mut Cursor::new(bytes)).unwrap(),
        key
    );
    let event = Event::Frame {
        id: 1,
        surface: Surface::DevTools,
        width: 2,
        height: 1,
        seq: 9,
    };
    let bgra = [1, 2, 3, 255, 4, 5, 6, 255];
    let mut bytes = vec![];
    protocol::write_event(&mut bytes, &event, &bgra).unwrap();
    let p = protocol::read_event(&mut Cursor::new(bytes)).unwrap();
    assert_eq!(p.message, event);
    assert_eq!(p.payload, bgra);
    let command = Command::DevToolsMessage { id: 1, epoch: 7 };
    let json = br#"{"id":2,"method":"Runtime.evaluate","params":{"expression":"21*2"}}"#;
    let mut bytes = vec![];
    protocol::write_command_packet(&mut bytes, &command, json).unwrap();
    let p = protocol::read_command_packet(&mut Cursor::new(bytes)).unwrap();
    assert_eq!(p.message, command);
    assert_eq!(p.payload, json);
}
#[test]
fn framing_unknown_fields_lengths_truncation_and_binary_rules_are_strict() {
    for header in [(0, 0), (65537, 0), (1, protocol::MAX_PAYLOAD as u32 + 1)] {
        let mut bytes = (header.0 as u32).to_le_bytes().to_vec();
        bytes.extend(header.1.to_le_bytes());
        assert!(protocol::read_event(&mut Cursor::new(bytes)).is_err());
    }
    for json in [
        br#"{"type":"Closed","id":1,"shell":"exec"}"#.as_slice(),
        br#"{"type":"Closed","id":1,"id":2}"#.as_slice(),
        br#"{"type":"Unknown"}"#.as_slice(),
        b"not JSON",
    ] {
        assert!(protocol::read_event(&mut Cursor::new(packet(json, &[]))).is_err());
    }
    let event = br#"{"type":"Closed","id":1}"#;
    assert!(protocol::read_event(&mut Cursor::new(packet(event, &[1]))).is_err());
    let mut bytes = packet(event, &[]);
    bytes.pop();
    assert!(protocol::read_event(&mut Cursor::new(bytes)).is_err());
    let command = br#"{"type":"Shutdown"}"#;
    assert!(protocol::read_command_packet(&mut Cursor::new(packet(command, &[1]))).is_err());
}
#[test]
fn invalid_dimensions_and_metadata_are_rejected_before_payload_is_read() {
    let json = br#"{"type":"Frame","id":1,"surface":"Page","width":8192,"height":8192,"seq":1}"#;
    let mut input = Cursor::new(packet(json, &[0; 4]));
    assert!(protocol::read_event(&mut input).is_err());
    assert_eq!(input.position() as usize, 8 + json.len());
    for (w, h) in [
        (0, 1),
        (1, 0),
        (8193, 1),
        (8192, 8192),
        (u32::MAX, u32::MAX),
    ] {
        assert!(protocol::frame_bytes(w, h).is_err());
    }
    for scale in [0., f32::NAN, f32::INFINITY, 0.4, 4.1] {
        assert!(protocol::physical_size(2, 2, scale).is_err());
    }
    assert_eq!(protocol::physical_size(641, 401, 1.25).unwrap(), (802, 502));
    let event = Event::Frame {
        id: 1,
        surface: Surface::Page,
        width: 2,
        height: 1,
        seq: 0,
    };
    assert!(protocol::write_event(&mut vec![], &event, &[0; 8]).is_err());
    let event = Event::State {
        id: 1,
        url: "file:///tmp/secret".into(),
        title: "text".into(),
        loading: false,
        can_go_back: false,
        can_go_forward: false,
        error: None,
    };
    assert!(protocol::write_event(&mut vec![], &event, &[]).is_err());
}
#[test]
fn ime_surrogate_boundaries_cdp_json_and_url_commands_are_bounded() {
    for (start, end) in [(-1, 0), (0, 3), (4, 2), (0, 5)] {
        let c = Command::Composition {
            id: 1,
            surface: Surface::Page,
            text: "e\u{301}😀".into(),
            selection_start: start,
            selection_end: end,
        };
        assert!(protocol::validate_command(&c).is_err(), "{start} {end}");
    }
    for url in [
        "file:///tmp/x",
        "javascript:alert(1)",
        "https://user:pass@example.com",
    ] {
        assert!(
            protocol::validate_command(&Command::Navigate {
                id: 1,
                url: url.into()
            })
            .is_err()
        );
    }
    for json in [
        b"[]".as_slice(),
        b"null".as_slice(),
        b"{broken".as_slice(),
        b"\xff".as_slice(),
    ] {
        assert!(protocol::validate_cdp(json).is_err());
    }
    assert!(protocol::validate_cdp(&vec![b' '; protocol::MAX_CDP + 1]).is_err());
    let c = Command::DevToolsMessage { id: 1, epoch: 0 };
    assert!(protocol::validate_command(&c).is_err());
}

#[test]
fn private_frontend_endpoint_is_validated_on_wire_and_debug_redacts_nonce() {
    use codisseu_browser::SecretUrl;
    let nonce = "a".repeat(64);
    let good = format!(
        "devtools://devtools/bundled/inspector.html?can_dock=false&ws=127.0.0.1:45678/{nonce}"
    );
    assert!(SecretUrl::devtools(good.replace("inspector.html", "devtools_app.html")).is_ok());
    let url = SecretUrl::devtools(good.clone()).unwrap();
    assert!(!format!("{url:?}").contains(&nonce));
    let command = Command::DevToolsFrontend { id: 1, url };
    let wire = serde_json::to_vec(&command).unwrap();
    assert_eq!(serde_json::from_slice::<Command>(&wire).unwrap(), command);
    for invalid in [
        good.replace("127.0.0.1", "localhost"),
        good.replace("45678", "80"),
        good.replace("inspector.html", "arbitrary.html"),
        format!("{good}&remoteFrontend=https://example.test"),
        format!("{good}#fragment"),
        good.replace(&nonce, &"A".repeat(64)),
        good.replace("can_dock=false&", ""),
    ] {
        assert!(SecretUrl::devtools(invalid.clone()).is_err());
        assert!(
            serde_json::from_value::<Command>(
                serde_json::json!({"type":"DevToolsFrontend","id":1,"url":invalid})
            )
            .is_err()
        );
    }
}

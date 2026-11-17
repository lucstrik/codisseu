//! Initial Phpactor index is bounded by inactivity, not project size.
use serde_json::Value;
use std::time::{Duration, Instant};

const IDLE: Duration = Duration::from_secs(120);
const TOTAL: Duration = Duration::from_secs(30 * 60);
pub(crate) struct IndexProgress {
    token: Option<Value>,
    started: Instant,
    activity: Instant,
}
pub(crate) enum Update {
    Ignored,
    Working(String),
    Finished,
}
impl IndexProgress {
    pub fn new(now: Instant) -> Self {
        Self {
            token: None,
            started: now,
            activity: now,
        }
    }
    pub fn receive(&mut self, params: &Value, now: Instant) -> Update {
        let Some(token) = params
            .get("token")
            .filter(|t| t.is_i64() || t.is_u64() || t.as_str().is_some_and(|s| s.len() <= 256))
        else {
            return Update::Ignored;
        };
        let kind = params.pointer("/value/kind").and_then(Value::as_str);
        if self.token.is_none()
            && kind == Some("begin")
            && params.pointer("/value/title").and_then(Value::as_str) == Some("Indexing workspace")
        {
            self.token = Some(token.clone());
        } else if self.token.as_ref() != Some(token) {
            return Update::Ignored;
        } else if kind == Some("end") {
            return Update::Finished;
        } else if kind != Some("report") {
            return Update::Ignored;
        }
        self.activity = now;
        let message: String = params
            .pointer("/value/message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect();
        Update::Working(if message.is_empty() {
            "Phpactor: indexando workspace".into()
        } else {
            format!("Phpactor: indexando workspace — {message}")
        })
    }
    pub fn timeout(&self, now: Instant) -> Option<&'static str> {
        if now.duration_since(self.activity) >= IDLE {
            Some(
                "Indexação Phpactor sem progresso por 120 segundos; reinicie o provider para tentar novamente",
            )
        } else if now.duration_since(self.started) >= TOTAL {
            Some("Indexação Phpactor excedeu o limite de 30 minutos")
        } else {
            None
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn event(token: &str, kind: &str) -> Value {
        json!({"token":token,"value":{"kind":kind,"title":"Indexing workspace","message":"500/1000"}})
    }
    #[test]
    fn active_large_index_survives_thirty_seconds_and_finishes_only_matching_token() {
        let now = Instant::now();
        let mut p = IndexProgress::new(now);
        assert!(matches!(
            p.receive(&event("a", "end"), now),
            Update::Ignored
        ));
        assert!(matches!(
            p.receive(&event("a", "begin"), now),
            Update::Working(_)
        ));
        for secs in [31, 90, 180, 270] {
            let at = now + Duration::from_secs(secs);
            assert!(matches!(
                p.receive(&event("a", "report"), at),
                Update::Working(_)
            ));
            assert_eq!(p.timeout(at), None);
        }
        assert!(matches!(
            p.receive(&event("b", "end"), now),
            Update::Ignored
        ));
        assert!(matches!(
            p.receive(&event("a", "end"), now),
            Update::Finished
        ));
    }
    #[test]
    fn unrelated_and_malformed_progress_do_not_extend_deadline() {
        let now = Instant::now();
        let mut p = IndexProgress::new(now);
        p.receive(&event("a", "begin"), now);
        for event in [
            event("b", "report"),
            event("a", "begin"),
            json!({"token":{},"value":{"kind":"report"}}),
            event("a", "bogus"),
        ] {
            assert!(matches!(
                p.receive(&event, now + Duration::from_secs(119)),
                Update::Ignored
            ));
        }
        assert!(
            p.timeout(now + Duration::from_secs(120))
                .unwrap()
                .contains("sem progresso")
        );
        assert!(
            IndexProgress::new(now)
                .timeout(now + Duration::from_secs(120))
                .is_some()
        );
    }
    #[test]
    fn progress_is_bounded_and_cannot_keep_provider_alive_forever() {
        let now = Instant::now();
        let mut p = IndexProgress::new(now);
        p.receive(&event("a", "begin"), now);
        let mut e = event("a", "report");
        e["value"]["message"] = Value::String("🌧\n".repeat(1000));
        let Update::Working(detail) = p.receive(&e, now + TOTAL) else {
            panic!()
        };
        assert!(detail.chars().count() < 250);
        assert!(!detail.contains('\n'));
        assert!(p.timeout(now + TOTAL).unwrap().contains("30 minutos"));
    }
}

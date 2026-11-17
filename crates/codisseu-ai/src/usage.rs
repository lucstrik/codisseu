//! Provider-reported counters. Token activity, context capacity and account quotas
//! are distinct; no prices or token allowance are inferred from percentages.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Breakdown {
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub total: Breakdown,
    pub last: Breakdown,
    pub model_context_window: Option<u64>,
}
impl Tokens {
    pub(crate) fn parse(value: &Value) -> Option<Self> {
        let mut tokens: Self = serde_json::from_value(value.clone()).ok()?;
        tokens.model_context_window = tokens.model_context_window.filter(|w| *w > 0);
        Some(tokens)
    }
    /// Based on the last reported call, not a remaining account token allowance.
    pub fn context_remaining(&self) -> Option<u64> {
        Some(
            self.model_context_window?
                .saturating_sub(self.last.total_tokens),
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub used_percent: f64,
    pub duration_mins: Option<u64>,
    pub resets_at: Option<u64>,
}
impl Window {
    fn parse(v: &Value) -> Option<Self> {
        let used_percent = v.get("usedPercent")?.as_f64()?;
        if !used_percent.is_finite() || used_percent < 0. {
            return None;
        }
        Some(Self {
            used_percent,
            duration_mins: v["windowDurationMins"].as_u64().filter(|v| *v > 0),
            resets_at: v["resetsAt"].as_u64().filter(|v| *v > 0),
        })
    }
    pub fn remaining(&self, now: u64) -> Option<f64> {
        if self.resets_at.is_some_and(|reset| reset <= now) {
            return None;
        }
        Some((100. - self.used_percent).clamp(0., 100.))
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Bucket {
    pub id: String,
    pub name: String,
    pub primary: Option<Window>,
    pub secondary: Option<Window>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    pub buckets: Vec<Bucket>,
    pub observed_at: u64,
}
impl Limits {
    pub(crate) fn parse(v: &Value) -> Option<Self> {
        let mut buckets = Vec::new();
        if let Some(map) = v["rateLimitsByLimitId"]
            .as_object()
            .filter(|map| !map.is_empty())
        {
            for (id, value) in map.iter().take(8) {
                buckets.push(bucket(id, value));
            }
        } else if let Some(value) = v.get("rateLimits").filter(|v| v.is_object()) {
            buckets.push(bucket(value["limitId"].as_str().unwrap_or("codex"), value));
        }
        if buckets.is_empty() {
            None
        } else {
            Some(Self {
                buckets,
                observed_at: now(),
            })
        }
    }
    /// Only the ordinary Codex bucket is summarized; other metered products are
    /// listed separately, never combined into an invented account-wide percentage.
    pub fn remaining(&self, now: u64) -> Option<f64> {
        if now.saturating_sub(self.observed_at) > 300 {
            return None;
        }
        let b = self
            .buckets
            .iter()
            .find(|b| b.id == "codex")
            .or_else(|| (self.buckets.len() == 1).then(|| &self.buckets[0]))?;
        let mut remaining: Option<f64> = None;
        for window in [&b.primary, &b.secondary].into_iter().flatten() {
            let available = window.remaining(now)?;
            remaining = Some(remaining.map_or(available, |current| current.min(available)));
        }
        remaining
    }
}
fn bucket(id: &str, v: &Value) -> Bucket {
    Bucket {
        id: id.chars().take(80).collect(),
        name: v["limitName"]
            .as_str()
            .unwrap_or(id)
            .chars()
            .take(80)
            .collect(),
        primary: Window::parse(&v["primary"]),
        secondary: Window::parse(&v["secondary"]),
    }
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn quota_prefers_multi_bucket_and_never_recovers_from_a_passed_reset() {
        let now = now();
        let v = json!({"rateLimits":{"primary":{"usedPercent":0}},"rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":28,"resetsAt":now+100},"secondary":{"usedPercent":64}},"other":{"primary":{"usedPercent":99}}}});
        let limits = Limits::parse(&v).unwrap();
        assert_eq!(limits.remaining(now), Some(36.));
        assert_eq!(limits.remaining(now + 101), None);
        assert_eq!(limits.remaining(now + 301), None);
        let w = Window::parse(&json!({"usedPercent":105,"resetsAt":now+1})).unwrap();
        assert_eq!(w.remaining(now), Some(0.));
        assert_eq!(w.remaining(now + 1), None);
        assert!(Window::parse(&json!({"usedPercent":null})).is_none());
        assert!(Limits::parse(&json!({})).is_none());
    }
    #[test]
    fn absent_negative_and_unknown_capacity_are_not_zero_usage() {
        let b = json!({"totalTokens":120,"inputTokens":100,"cachedInputTokens":50,"outputTokens":20,"reasoningOutputTokens":10});
        let mut v = json!({"total":b,"last":b,"modelContextWindow":1000});
        let t = Tokens::parse(&v).unwrap();
        assert_eq!(t.context_remaining(), Some(880));
        assert_eq!(t.total.cached_input_tokens, 50);
        v["modelContextWindow"] = Value::Null;
        assert_eq!(Tokens::parse(&v).unwrap().context_remaining(), None);
        v["total"]["totalTokens"] = json!(-1);
        assert!(Tokens::parse(&v).is_none());
        assert!(Tokens::parse(&Value::Null).is_none());
    }
}

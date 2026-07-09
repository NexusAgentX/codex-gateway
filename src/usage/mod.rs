use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct UsageSnapshot {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub source: UsageSource,
    pub upstream_response_id: Option<String>,
    pub upstream_status: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    Upstream,
    Estimated,
    #[default]
    Unknown,
}

impl UsageSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Upstream => "upstream",
            Self::Estimated => "estimated",
            Self::Unknown => "unknown",
        }
    }
}

pub fn extract_usage_from_json(value: &Value) -> UsageSnapshot {
    let usage = value
        .pointer("/usage")
        .or_else(|| value.pointer("/response/usage"));
    let mut snapshot = usage_to_snapshot(usage);
    if snapshot.source == UsageSource::Upstream {
        snapshot.upstream_response_id = value
            .pointer("/id")
            .or_else(|| value.pointer("/response/id"))
            .and_then(Value::as_str)
            .map(str::to_string);
        snapshot.upstream_status = value
            .pointer("/status")
            .or_else(|| value.pointer("/response/status"))
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    snapshot
}

fn usage_to_snapshot(usage: Option<&Value>) -> UsageSnapshot {
    let Some(usage) = usage else {
        return UsageSnapshot::default();
    };
    let prompt_tokens = usage
        .get("input_tokens")
        .or_else(|| usage.get("prompt_tokens"))
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let completion_tokens = usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let total_tokens = usage
        .get("total_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(prompt_tokens + completion_tokens);

    UsageSnapshot {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        source: UsageSource::Upstream,
        upstream_response_id: None,
        upstream_status: None,
    }
}

#[derive(Debug, Default)]
pub struct SseUsageScanner {
    pending: String,
    snapshot: UsageSnapshot,
}

impl SseUsageScanner {
    pub fn push(&mut self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        self.pending.push_str(&text);
        while let Some(index) = self.pending.find('\n') {
            let line = self.pending[..index].trim_end_matches('\r').to_string();
            self.pending.drain(..=index);
            self.process_line(&line);
        }
        if self.pending.len() > 64 * 1024 {
            self.pending.clear();
        }
    }

    pub fn snapshot(&self) -> UsageSnapshot {
        self.snapshot.clone()
    }

    fn process_line(&mut self, line: &str) {
        let Some(data) = line.strip_prefix("data:") else {
            return;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };
        let candidate = if value.get("type").and_then(Value::as_str) == Some("response.completed") {
            let mut snapshot = extract_usage_from_json(&value);
            if snapshot.source != UsageSource::Upstream {
                snapshot = usage_to_snapshot(value.pointer("/response/usage"));
            }
            snapshot.upstream_response_id = value
                .pointer("/response/id")
                .and_then(Value::as_str)
                .map(str::to_string);
            snapshot.upstream_status = value
                .pointer("/response/status")
                .and_then(Value::as_str)
                .map(str::to_string);
            snapshot
        } else {
            extract_usage_from_json(&value)
        };

        if candidate.source == UsageSource::Upstream {
            self.snapshot = candidate;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_responses_usage() {
        let value = serde_json::json!({
            "id": "resp_1",
            "status": "completed",
            "usage": {
                "input_tokens": 3,
                "output_tokens": 4,
                "total_tokens": 7
            }
        });
        let usage = extract_usage_from_json(&value);
        assert_eq!(usage.prompt_tokens, 3);
        assert_eq!(usage.completion_tokens, 4);
        assert_eq!(usage.total_tokens, 7);
        assert_eq!(usage.source, UsageSource::Upstream);
    }

    #[test]
    fn scans_sse_completed_event() {
        let mut scanner = SseUsageScanner::default();
        scanner.push(br#"data: {"type":"response.completed","response":{"id":"resp_2","status":"completed","usage":{"input_tokens":2,"output_tokens":5,"total_tokens":7}}}"#);
        scanner.push(b"\n\n");
        let usage = scanner.snapshot();
        assert_eq!(usage.total_tokens, 7);
        assert_eq!(usage.upstream_response_id.as_deref(), Some("resp_2"));
    }
}

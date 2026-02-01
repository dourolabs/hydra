use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const MAX_LINE_LEN: usize = 160;
const TOOL_SNIPPET_LEN: usize = 400;
const TEXT_BLOCK_GAP: &str = "\n";

struct PendingToolCall {
    name: String,
    summary: String,
    started_at: Instant,
}

impl PendingToolCall {
    fn new(name: String, summary: String) -> Self {
        Self {
            name,
            summary,
            started_at: Instant::now(),
        }
    }
}

pub struct StreamFormatter {
    pending_tools: HashMap<String, PendingToolCall>,
}

impl StreamFormatter {
    pub fn new() -> Self {
        Self {
            pending_tools: HashMap::new(),
        }
    }

    pub fn handle_line(&mut self, raw_line: &str) -> Vec<String> {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            return vec![];
        }

        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            return vec![];
        };

        match value.get("type").and_then(Value::as_str) {
            Some("assistant") => self.handle_assistant(&value),
            Some("user") => self.handle_user(&value),
            _ => vec![],
        }
    }

    fn handle_assistant(&mut self, value: &Value) -> Vec<String> {
        let mut renders = Vec::new();
        let Some(content) = value
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
        else {
            return renders;
        };

        for chunk in content {
            let Some(chunk_type) = chunk.get("type").and_then(Value::as_str) else {
                continue;
            };
            match chunk_type {
                "text" => {
                    if let Some(text) = chunk.get("text").and_then(Value::as_str) {
                        renders.push(format_block("assistant>", text));
                    }
                }
                "thinking" | "reasoning" => {
                    if let Some(text) = chunk.get("text").and_then(Value::as_str) {
                        renders.push(format_block("reasoning>", text));
                    }
                }
                "tool_use" => {
                    if let Some(rendered) = self.handle_tool_use(chunk) {
                        renders.push(rendered);
                    }
                }
                _ => {}
            }
        }

        renders
    }

    fn handle_tool_use(&mut self, chunk: &Value) -> Option<String> {
        let id = chunk.get("id").and_then(Value::as_str)?.to_owned();
        let name = chunk
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("tool")
            .to_owned();
        let summary = summarize_input(chunk.get("input"));
        self.pending_tools
            .insert(id, PendingToolCall::new(name.clone(), summary.clone()));

        Some(format!("tool> {name} - {summary}{TEXT_BLOCK_GAP}"))
    }

    fn handle_user(&mut self, value: &Value) -> Vec<String> {
        let mut renders = Vec::new();
        let Some(content) = value
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
        else {
            return renders;
        };

        for chunk in content {
            let Some("tool_result") = chunk.get("type").and_then(Value::as_str) else {
                continue;
            };
            let Some(tool_id) = chunk.get("tool_use_id").and_then(Value::as_str) else {
                continue;
            };
            let result_text = extract_tool_result_text(chunk.get("content"));
            let is_error = chunk
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if let Some(pending) = self.pending_tools.remove(tool_id) {
                let duration = pending.started_at.elapsed();
                renders.push(format_tool_result(
                    &pending.name,
                    &pending.summary,
                    duration,
                    &result_text,
                    is_error,
                ));
            }
        }

        renders
    }
}

fn format_block(prefix: &str, text: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }

    let mut rendered = String::new();
    for line in text.replace('\r', "").lines() {
        if line.trim().is_empty() {
            rendered.push('\n');
            continue;
        }
        rendered.push_str(prefix);
        rendered.push(' ');
        rendered.push_str(&truncate(line, MAX_LINE_LEN));
        rendered.push('\n');
    }
    rendered.push_str(TEXT_BLOCK_GAP);
    rendered
}

fn summarize_input(value: Option<&Value>) -> String {
    let summary = match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Object(map)) => map
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                map.get("command")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| serde_json::to_string(map).unwrap_or_else(|_| "input".to_string())),
        Some(other) => serde_json::to_string(other).unwrap_or_else(|_| String::new()),
        None => String::new(),
    };

    if summary.is_empty() {
        "started".to_string()
    } else {
        truncate(&summary, MAX_LINE_LEN)
    }
}

fn extract_tool_result_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => truncate(text, TOOL_SNIPPET_LEN),
        Some(Value::Array(items)) => truncate(
            &items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
                .join("\n"),
            TOOL_SNIPPET_LEN,
        ),
        Some(other) => truncate(
            &serde_json::to_string(other).unwrap_or_else(|_| String::new()),
            TOOL_SNIPPET_LEN,
        ),
        None => String::new(),
    }
}

fn format_tool_result(
    name: &str,
    summary: &str,
    duration: Duration,
    content: &str,
    is_error: bool,
) -> String {
    let mut rendered = String::new();
    let status = if is_error {
        "tool error>"
    } else {
        "tool done>"
    };
    rendered.push_str(&format!(
        "{status} {name} - {summary} ({:.1}s)\n",
        duration.as_secs_f32()
    ));
    if !content.trim().is_empty() {
        for line in content.lines() {
            rendered.push_str("  ");
            rendered.push_str(&truncate(line, MAX_LINE_LEN));
            rendered.push('\n');
        }
    }
    rendered.push_str(TEXT_BLOCK_GAP);
    rendered
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    text.chars()
        .take(limit.saturating_sub(3))
        .collect::<String>()
        + "..."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_input_prefers_description_then_command() {
        let mut map = serde_json::Map::new();
        map.insert("description".to_string(), Value::String("describe".into()));
        map.insert("command".to_string(), Value::String("cmd".into()));
        assert_eq!(
            summarize_input(Some(&Value::Object(map.clone()))),
            "describe"
        );

        map.remove("description");
        assert_eq!(
            summarize_input(Some(&Value::Object(map))),
            "cmd".to_string()
        );
    }

    #[test]
    fn format_block_prefixes_each_line() {
        let rendered = format_block("assistant>", "hello\nworld");
        assert!(rendered.contains("assistant> hello"));
        assert!(rendered.contains("assistant> world"));
    }

    #[test]
    fn extract_tool_result_handles_string_and_array() {
        let string_value = Value::String("first line\nsecond".into());
        assert!(extract_tool_result_text(Some(&string_value)).contains("first line"));

        let array_value = Value::Array(vec![
            Value::String("alpha".into()),
            Value::String("beta".into()),
        ]);
        let rendered = extract_tool_result_text(Some(&array_value));
        assert!(rendered.contains("alpha"));
        assert!(rendered.contains("beta"));
    }

    #[test]
    fn format_tool_result_includes_duration_and_status() {
        let rendered = format_tool_result(
            "Bash",
            "run command",
            Duration::from_millis(1500),
            "ok",
            false,
        );
        assert!(rendered.contains("tool done> Bash - run command (1.5s)"));
        assert!(rendered.contains("ok"));
    }

    #[test]
    fn truncate_adds_ellipsis_when_needed() {
        let result = truncate("abcdef", 5);
        assert_eq!(result, "ab...");
    }
}

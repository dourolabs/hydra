use owo_colors::OwoColorize;
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
        let input = chunk.get("input");
        let summary = summarize_input(input);
        self.pending_tools
            .insert(id, PendingToolCall::new(name.clone(), summary.clone()));

        // For Bash tool, show full command on a separate line
        if name == "Bash" {
            let command = input
                .and_then(Value::as_object)
                .and_then(|obj| obj.get("command"))
                .and_then(Value::as_str)
                .unwrap_or("");
            Some(format!(
                "{} {name} - {summary}\n  $ {command}{TEXT_BLOCK_GAP}",
                "tool>".yellow()
            ))
        } else {
            Some(format!(
                "{} {name} - {summary}{TEXT_BLOCK_GAP}",
                "tool>".yellow()
            ))
        }
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

    let colored_prefix = match prefix {
        "assistant>" => prefix.cyan().to_string(),
        "reasoning>" => prefix.dimmed().to_string(),
        _ => prefix.to_string(),
    };

    let mut rendered = String::new();
    for line in text.replace('\r', "").lines() {
        if line.trim().is_empty() {
            rendered.push('\n');
            continue;
        }
        rendered.push_str(&colored_prefix);
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
        "tool error>".red().to_string()
    } else {
        "tool done>".green().to_string()
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
        // Output contains colored prefix (cyan for assistant>) plus content
        assert!(rendered.contains("assistant>"));
        assert!(rendered.contains(" hello"));
        assert!(rendered.contains(" world"));
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
        // Output contains colored prefix (green for tool done>) plus content
        assert!(rendered.contains("tool done>"));
        assert!(rendered.contains("Bash - run command (1.5s)"));
        assert!(rendered.contains("ok"));
    }

    #[test]
    fn truncate_adds_ellipsis_when_needed() {
        let result = truncate("abcdef", 5);
        assert_eq!(result, "ab...");
    }

    #[test]
    fn format_block_uses_cyan_for_assistant_prefix() {
        let rendered = format_block("assistant>", "test");
        // Cyan ANSI code is \x1b[36m
        assert!(rendered.contains("\x1b[36m"));
        assert!(rendered.contains("assistant>"));
    }

    #[test]
    fn format_block_uses_dim_for_reasoning_prefix() {
        let rendered = format_block("reasoning>", "test");
        // Dim ANSI code is \x1b[2m
        assert!(rendered.contains("\x1b[2m"));
        assert!(rendered.contains("reasoning>"));
    }

    #[test]
    fn format_tool_result_uses_green_for_success() {
        let rendered = format_tool_result("Test", "summary", Duration::from_secs(1), "", false);
        // Green ANSI code is \x1b[32m
        assert!(rendered.contains("\x1b[32m"));
        assert!(rendered.contains("tool done>"));
    }

    #[test]
    fn format_tool_result_uses_red_for_error() {
        let rendered = format_tool_result("Test", "summary", Duration::from_secs(1), "", true);
        // Red ANSI code is \x1b[31m
        assert!(rendered.contains("\x1b[31m"));
        assert!(rendered.contains("tool error>"));
    }

    #[test]
    fn handle_tool_use_shows_full_bash_command_on_separate_line() {
        let mut formatter = StreamFormatter::new();
        let chunk = serde_json::json!({
            "type": "tool_use",
            "id": "tool_1",
            "name": "Bash",
            "input": {
                "description": "List directory contents",
                "command": "ls -la /tmp"
            }
        });
        let rendered = formatter.handle_tool_use(&chunk).unwrap();
        // Check that tool> prefix is yellow (ANSI code \x1b[33m)
        assert!(rendered.contains("\x1b[33m"));
        assert!(rendered.contains("tool>"));
        // Check that description appears on first line
        assert!(rendered.contains("Bash - List directory contents"));
        // Check that full command appears on separate indented line with $ prefix
        assert!(rendered.contains("\n  $ ls -la /tmp"));
    }

    #[test]
    fn handle_tool_use_bash_does_not_truncate_long_commands() {
        let mut formatter = StreamFormatter::new();
        let long_command = "find /very/long/path/to/some/directory -name '*.rs' -exec grep -l 'pattern' {} \\; | xargs sed -i 's/old/new/g'";
        let chunk = serde_json::json!({
            "type": "tool_use",
            "id": "tool_2",
            "name": "Bash",
            "input": {
                "description": "Find and replace in files",
                "command": long_command
            }
        });
        let rendered = formatter.handle_tool_use(&chunk).unwrap();
        // Full command should appear without truncation
        assert!(rendered.contains(&format!("$ {long_command}")));
    }

    #[test]
    fn handle_tool_use_non_bash_tools_unchanged() {
        let mut formatter = StreamFormatter::new();
        let chunk = serde_json::json!({
            "type": "tool_use",
            "id": "tool_3",
            "name": "Read",
            "input": {
                "file_path": "/path/to/file.rs"
            }
        });
        let rendered = formatter.handle_tool_use(&chunk).unwrap();
        // Check that non-Bash tools don't have the "$ " command line
        assert!(!rendered.contains("  $ "));
        assert!(rendered.contains("Read - "));
    }
}

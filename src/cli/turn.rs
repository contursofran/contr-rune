//! What a Claude Code transcript (JSON lines) says: the last turn, and the model of the last
//! assistant message. Go: internal/agents/turn.go (`LastTurn`, `ContextAndModel`). A line that is
//! not JSON is skipped: the format belongs to Claude Code, so only the needed fields are read.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

/// The part of the transcript after the last prompt.
#[derive(Debug, Default, PartialEq)]
pub struct Turn {
    /// The turn sent a message to another terminal: a Bash call with `hc send`, or the kit's
    /// `message` tool.
    pub reported: bool,
    /// The last text of the assistant in the turn.
    pub said: String,
}

/// The last turn of the transcript `text`.
pub fn last_turn(text: &[u8]) -> Turn {
    let mut turn = Turn::default();
    for line in text.split(|&b| b == b'\n') {
        let Ok(entry) = serde_json::from_slice::<Value>(line) else { continue };
        let content = &entry["message"]["content"];
        match entry["type"].as_str() {
            Some("user") if entry["isMeta"] != Value::Bool(true) && is_prompt(content) => turn = Turn::default(),
            Some("assistant") => {
                for block in content.as_array().into_iter().flatten() {
                    match block["type"].as_str() {
                        Some("text") => {
                            let said = block["text"].as_str().unwrap_or_default().trim();
                            if !said.is_empty() {
                                turn.said = said.to_owned();
                            }
                        }
                        Some("tool_use") if reports(block) => turn.reported = true,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    turn
}

/// User content is a prompt (typed or sent text), not a tool result.
fn is_prompt(content: &Value) -> bool {
    match content {
        Value::String(_) => true,
        Value::Array(blocks) => !blocks.is_empty() && blocks.iter().all(|b| b["type"] != "tool_result"),
        _ => false,
    }
}

/// A tool call that sends a message to another terminal.
fn reports(block: &Value) -> bool {
    let name = block["name"].as_str().unwrap_or_default();
    if name == "Bash" {
        return block["input"]["command"].as_str().is_some_and(|c| c.contains("hc send"));
    }
    name.ends_with("contr__message")
}

/// How much of the end of a transcript `last_model` reads.
const TAIL_BYTES: u64 = 1 << 20;

/// The model id of the last assistant message in the last `TAIL_BYTES` of the transcript at
/// `path`, or `None`. Claude Code writes `<synthetic>` for messages it made itself: skipped.
pub fn last_model(path: &Path) -> std::io::Result<Option<String>> {
    let mut file = std::fs::File::open(path)?;
    let size = file.metadata()?.len();
    if size > TAIL_BYTES {
        file.seek(SeekFrom::Start(size - TAIL_BYTES))?;
    }
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)?;
    let mut model = None;
    // The first line after a seek is cut; it does not parse and is skipped.
    for line in tail.split(|&b| b == b'\n') {
        let Ok(entry) = serde_json::from_slice::<Value>(line) else { continue };
        if entry["type"] != "assistant" {
            continue;
        }
        if let Some(m) = entry["message"]["model"].as_str().filter(|m| !m.is_empty() && !m.starts_with('<')) {
            model = Some(m.to_owned());
        }
    }
    Ok(model)
}

/// The family word of a model id: `sonnet` for `claude-sonnet-5-5`; an unknown id stays as it is.
pub fn short_model(id: &str) -> String {
    let lower = id.to_lowercase();
    ["opus", "sonnet", "haiku"].into_iter().find(|f| lower.contains(f)).map_or_else(|| id.to_owned(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn_of(lines: &[&str]) -> Turn {
        last_turn(lines.join("\n").as_bytes())
    }

    const PROMPT: &str = r#"{"type":"user","message":{"role":"user","content":"do it"}}"#;
    const SAY: &str = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"  working  "}]}}"#;
    const RESULT: &str = r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}"#;
    const BASH: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"hc send t3 done"}}]}}"#;

    #[test]
    fn last_turn_table() {
        let said = |s: &str| Turn { reported: false, said: s.to_owned() };
        let cases: Vec<(&str, Vec<&str>, Turn)> = vec![
            ("empty", vec![], Turn::default()),
            ("text only", vec![PROMPT, SAY], said("working")),
            ("hc send reports", vec![PROMPT, BASH, RESULT, SAY], Turn { reported: true, said: "working".into() }),
            ("a new prompt resets the turn", vec![PROMPT, BASH, SAY, PROMPT], Turn::default()),
            ("a tool result is not a prompt", vec![PROMPT, SAY, RESULT], said("working")),
            ("meta user lines are not prompts", vec![PROMPT, SAY, r#"{"type":"user","isMeta":true,"message":{"content":"x"}}"#], said("working")),
            ("a prompt in blocks is a prompt", vec![SAY, r#"{"type":"user","message":{"content":[{"type":"text","text":"hi"}]}}"#], Turn::default()),
            (
                "the kit message tool reports",
                vec![PROMPT, r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__plugin_contr_contr__message","input":{}}]}}"#],
                Turn { reported: true, said: String::new() },
            ),
            ("other Bash does not report", vec![PROMPT, r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#], Turn::default()),
            ("garbage lines are skipped", vec![PROMPT, "not json", "", SAY], said("working")),
            ("blank text keeps the last words", vec![PROMPT, SAY, r#"{"type":"assistant","message":{"content":[{"type":"text","text":"  "}]}}"#], said("working")),
        ];
        for (name, lines, want) in cases {
            assert_eq!(turn_of(&lines), want, "{name}");
        }
    }

    #[test]
    fn short_model_table() {
        for (id, want) in [("claude-opus-5-5", "opus"), ("claude-sonnet-5-5", "sonnet"), ("Claude-Haiku-4", "haiku"), ("gpt-9", "gpt-9")] {
            assert_eq!(short_model(id), want);
        }
    }

    #[test]
    fn last_model_reads_the_last_real_model() {
        let dir = std::env::temp_dir().join(format!("contr-rune-turn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        let line = |m: &str| format!("{{\"type\":\"assistant\",\"message\":{{\"model\":\"{m}\"}}}}\n");
        std::fs::write(&path, format!("{}oops\n{}{}", line("claude-opus-5-5"), line("claude-sonnet-5-5"), line("<synthetic>"))).unwrap();
        assert_eq!(last_model(&path).unwrap().as_deref(), Some("claude-sonnet-5-5"));
        std::fs::write(&path, PROMPT).unwrap();
        assert_eq!(last_model(&path).unwrap(), None);
        assert!(last_model(&dir.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

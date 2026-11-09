//! Serde types for the hc JSON-RPC wire (Contract C1 to C4). Only the fields contr-rune reads.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Result of `hello`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct HelloResult {
    #[serde(default)]
    pub generation: u32,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub framed: bool,
    #[serde(default)]
    pub core: Core,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Core {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// Result of `server.status`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct ServerStatus {
    #[serde(default)]
    pub core: Core,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub pid: u64,
    #[serde(default)]
    pub uptime_ms: u64,
    #[serde(default)]
    pub live: u64,
    #[serde(default)]
    pub finished: u64,
}

/// What a terminal's program reported with OSC 7501.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Status {
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub msg: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub progress: Option<Value>,
}

/// The foreground process of a terminal.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Foreground {
    #[serde(default)]
    pub pid: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub argv: Vec<String>,
    #[serde(default)]
    pub cwd: String,
}

/// A terminal record (C3).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Terminal {
    pub address: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub argv: Vec<String>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub turn: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub created_ms: u64,
    #[serde(default)]
    pub status_ms: u64,
    #[serde(default)]
    pub cols: u16,
    #[serde(default)]
    pub rows: u16,
    #[serde(default)]
    pub foreground: Option<Foreground>,
    #[serde(default)]
    pub status: Option<Status>,
}

/// Result of `terminal.list`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct TerminalList {
    #[serde(default)]
    pub terminals: Vec<Terminal>,
}

/// Result of `events.subscribe`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Snapshot {
    #[serde(default)]
    pub seq: u64,
    #[serde(default)]
    pub terminals: Vec<Terminal>,
    #[serde(default)]
    pub channel: Option<u32>,
}

/// An `event` notification's params (C4).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Event {
    pub seq: u64,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub terminal: Option<String>,
    #[serde(default)]
    pub data: Value,
}

/// A `missed` notification's params: a fresh snapshot for a subscriber that fell behind.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Missed {
    #[serde(default)]
    pub seq: u64,
    #[serde(default)]
    pub terminals: Vec<Terminal>,
}

/// An error object: `{code, message, data{kind, hint}}`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct RpcError {
    #[serde(default)]
    pub code: i64,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub data: Option<ErrorData>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct ErrorData {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub hint: Option<String>,
}

impl RpcError {
    pub fn kind(&self) -> &str {
        self.data.as_ref().map_or("", |d| d.kind.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_terminal_reads_its_c3_fields_and_ignores_the_rest() {
        let t: Terminal = serde_json::from_value(json!({
            "address": "t17", "name": "b-241", "parent": "t3", "cwd": "/x", "argv": ["claude", "--model", "sonnet"],
            "labels": {"contr": "agent", "role": "builder"}, "state": "live", "turn": 4, "exit_code": null,
            "foreground": {"pid": 1, "name": "claude", "argv": [], "cwd": "/x"},
            "status": {"state": "blocked", "kind": "permission", "msg": "ok?", "title": "", "app": "claude-code", "progress": null},
            "something_new": 1
        }))
        .unwrap();
        assert_eq!(t.address, "t17");
        assert_eq!(t.parent.as_deref(), Some("t3"));
        assert_eq!(t.labels["role"], "builder");
        assert_eq!(t.status.unwrap().kind.as_deref(), Some("permission"));
    }

    #[test]
    fn an_error_exposes_its_kind() {
        let e: RpcError = serde_json::from_value(json!({"code": -32000, "message": "no", "data": {"kind": "not_found", "hint": "h"}})).unwrap();
        assert_eq!(e.kind(), "not_found");
        assert_eq!(RpcError::default().kind(), "");
    }

    #[test]
    fn an_event_reads_its_type() {
        let e: Event = serde_json::from_value(json!({"seq": 3, "type": "turn", "terminal": "t7", "data": {"turn": 2}})).unwrap();
        assert_eq!((e.seq, e.kind.as_str(), e.terminal.as_deref()), (3, "turn", Some("t7")));
    }
}

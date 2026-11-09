//! The menu a blocked worker shows on its terminal (port of go:internal/prompt). Pure: it takes
//! screen text and returns the question and the numbered choices. Choice `n` (from 1) is picked
//! with the digit key `n`.

use std::path::Path;

/// The most choices one digit key can pick.
const MAX_CHOICES: usize = 9;

/// The most lines `detail` looks above the question for the top of the menu's box.
const MAX_DETAIL: usize = 12;

#[derive(Debug, Clone, PartialEq)]
pub struct Prompt {
    /// The lines of the menu's box above the question (for a permission: the tool and its
    /// command). Empty when the menu has no box or separator above it.
    pub detail: Vec<String>,
    pub question: String,
    pub choices: Vec<String>,
}

/// The last menu on `screen`. `command` is the worker's program, `kind` the blocked kind
/// (`permission`, `question`, `auth`). `None` for an unknown program, an auth prompt or an
/// unknown form.
pub fn parse(command: &str, kind: &str, screen: &str) -> Option<Prompt> {
    if kind == "auth" {
        return None;
    }
    let program = command.split_whitespace().next().and_then(|c| Path::new(c).file_name())?;
    if !matches!(program.to_str(), Some("claude" | "codex")) {
        return None;
    }
    parse_menu(screen)
}

/// `N. label`: `(N, label)`.
fn item(line: &str) -> Option<(usize, &str)> {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    let rest = line[digits..].strip_prefix('.')?;
    let label = rest.trim_start();
    if digits == 0 || label.len() == rest.len() || label.is_empty() {
        return None;
    }
    Some((line[..digits].parse().ok()?, label.trim_end()))
}

/// A screen line without box characters and the cursor mark.
fn clean(line: &str) -> &str {
    let line = line.trim_matches(|c: char| " \t\r│┃╭╮╰╯─━".contains(c));
    for mark in ["❯", "›", ">"] {
        if let Some(rest) = line.strip_prefix(mark) {
            return rest.trim();
        }
    }
    line.trim()
}

/// The last run of numbered items that counts up from 1, with the nearest text line above it.
fn parse_menu(screen: &str) -> Option<Prompt> {
    let raw: Vec<&str> = screen.split('\n').collect();
    let lines: Vec<&str> = raw.iter().map(|l| clean(l)).collect();
    let items: Vec<(usize, usize, &str)> = lines.iter().enumerate().filter_map(|(i, l)| item(l).map(|(n, label)| (i, n, label))).collect();
    // Walk back from the last item until the numbers reach 1.
    let mut start = items.len().checked_sub(1)?;
    while items[start].1 != 1 {
        if start == 0 || items[start - 1].1 + 1 != items[start].1 {
            return None;
        }
        start -= 1;
    }
    let run = &items[start..];
    if run.len() < 2 || run.len() > MAX_CHOICES {
        return None;
    }
    let q = (0..run[0].0).rev().find(|&i| !lines[i].is_empty());
    let question = q.map(|i| lines[i].to_owned()).unwrap_or_default();
    let detail = q.filter(|&q| q > 0).map(|q| detail(&raw, &lines, q)).unwrap_or_default();
    Some(Prompt { detail, question, choices: run.iter().map(|r| r.2.to_owned()).collect() })
}

/// The cleaned lines between the top of the menu's box (a `╭` line or a line of `─`) and the
/// question line `q`. With no top within `MAX_DETAIL` lines, or agent output (`●`) or another
/// item in between, there is no detail.
fn detail(raw: &[&str], lines: &[&str], q: usize) -> Vec<String> {
    for i in (q.saturating_sub(MAX_DETAIL)..q).rev() {
        let r = raw[i].trim();
        if r.starts_with('╭') || (!r.is_empty() && r.trim_matches(|c| c == '─' || c == '━').is_empty()) {
            return lines[i + 1..q].iter().filter(|l| !l.is_empty()).map(|l| (*l).to_owned()).collect();
        }
        if lines[i].starts_with('●') || item(lines[i]).is_some() {
            break;
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERMISSION: &str = "● Running tests\n\n╭──────────────────────────────╮\n│ Bash command                 │\n│   cargo test                 │\n│                              │\n│ Do you want to proceed?      │\n│ ❯ 1. Yes                     │\n│   2. Yes, and don't ask again │\n│   3. No, and tell Claude what to do differently │\n╰──────────────────────────────╯\n";

    fn menu(detail: &[&str], question: &str, choices: &[&str]) -> Option<Prompt> {
        Some(Prompt { detail: detail.iter().map(|s| (*s).into()).collect(), question: question.into(), choices: choices.iter().map(|s| (*s).into()).collect() })
    }

    #[test]
    fn reads_the_last_menu() {
        for (name, command, kind, screen, want) in [
            ("permission box", "claude", "permission", PERMISSION, menu(&["Bash command", "cargo test"], "Do you want to proceed?", &["Yes", "Yes, and don't ask again", "No, and tell Claude what to do differently"])),
            ("path and codex", "/usr/local/bin/codex --x", "permission", "Allow it?\n› 1. Yes\n  2. No\n", menu(&[], "Allow it?", &["Yes", "No"])),
            ("an older menu above is ignored", "claude", "question", "Pick\n1. a\n2. b\nthen\nPick again\n1. c\n2. d\n3. e\n", menu(&[], "Pick again", &["c", "d", "e"])),
            ("a separator line starts the detail", "claude", "permission", "────────\nEdit file\nsrc/a.rs\nProceed?\n1. Yes\n2. No\n", menu(&["Edit file", "src/a.rs"], "Proceed?", &["Yes", "No"])),
            ("agent output stops the detail", "claude", "permission", "────────\n● Doing\nProceed?\n1. Yes\n2. No\n", menu(&[], "Proceed?", &["Yes", "No"])),
            ("no question above", "claude", "question", "1. a\n2. b\n", menu(&[], "", &["a", "b"])),
            ("auth", "claude", "auth", "Log in?\n1. a\n2. b\n", None),
            ("unknown program", "zsh", "question", "Q?\n1. a\n2. b\n", None),
            ("no command", "", "question", "Q?\n1. a\n2. b\n", None),
            ("no menu", "claude", "question", "just text\n", None),
            ("one item", "claude", "question", "Q?\n1. a\n", None),
            ("does not start at 1", "claude", "question", "Q?\n2. a\n3. b\n", None),
            ("a gap in the numbers", "claude", "question", "Q?\n1. a\n3. b\n", None),
            ("ten choices", "claude", "question", "Q?\n1. a\n2. a\n3. a\n4. a\n5. a\n6. a\n7. a\n8. a\n9. a\n10. a\n", None),
            ("a number with no label", "claude", "question", "Q?\n1.\n2. b\n", None),
            ("a number with no space", "claude", "question", "Q?\n1.a\n2.b\n", None),
        ] {
            assert_eq!(parse(command, kind, screen), want, "{name}");
        }
    }
}

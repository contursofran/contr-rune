//! Settings → Agents → Roles (C7, Go D186): the `orchestrator`, `planner`, `builder`, `reviewer`
//! and `debugger` profiles of `agents.json`, each with CLI, Model and Effort `Select`s and the
//! argv line they make. A pick sets `--model` or `--effort` and keeps the other arguments, then
//! saves through `agents_config` (unknown keys kept). A profile the selects cannot show reads
//! `Custom` and is not written until a pick. A missing orchestrator reads `not set` with a
//! suggestion; a pick or the Set button creates it.
//!
//! Keys: `role/<profile>/cli|model|effort|argv`, `role/<profile>/set` (a missing profile),
//! `roles/error`. The Flow section (`flow_section`) follows the Roles.

use rune::{El, col, el, row, spacer, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, Select, SelectAppearance};

use crate::App;
use crate::agents_config::{Config, Flow, Profile, flag_value};

/// The profiles the tab edits, in order.
const ROLES: [&str; 5] = ["orchestrator", "planner", "builder", "reviewer", "debugger"];
const CLAUDE: &str = "Claude Code";
const MODELS: [&str; 3] = ["opus", "sonnet", "haiku"];
const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
/// What a missing profile starts as.
const SUGGESTED: [&str; 7] = ["claude", "--model", "opus", "--effort", "medium", "--permission-mode", "auto"];
/// The width of one select.
const SELECT_W: f32 = 128.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Cli,
    Model,
    Effort,
}

impl Field {
    const ALL: [Field; 3] = [Field::Cli, Field::Model, Field::Effort];

    fn key(self) -> &'static str {
        match self {
            Field::Cli => "cli",
            Field::Model => "model",
            Field::Effort => "effort",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Field::Cli => "CLI",
            Field::Model => "Model",
            Field::Effort => "Effort",
        }
    }

    fn options(self) -> &'static [&'static str] {
        match self {
            Field::Cli => &[CLAUDE],
            Field::Model => &MODELS,
            Field::Effort => &EFFORTS,
        }
    }

    fn flag(self) -> &'static str {
        if self == Field::Model { "--model" } else { "--effort" }
    }
}

fn suggested() -> Profile {
    Profile { argv: SUGGESTED.map(String::from).to_vec(), agent: String::new() }
}

fn is_claude(p: &Profile) -> bool {
    p.argv.first().map(String::as_str) == Some("claude")
}

/// What the select of `field` shows for the profile: `None` for a missing profile or flag,
/// `Custom` when the argv is not one the options can name.
fn shown(p: Option<&Profile>, field: Field) -> Option<String> {
    let p = p?;
    if !is_claude(p) {
        return Some("Custom".into());
    }
    if field == Field::Cli {
        return Some(CLAUDE.into());
    }
    let v = flag_value(&p.argv, field.flag());
    match v.as_str() {
        "" => None,
        v if field.options().contains(&v) => Some(v.into()),
        _ => Some("Custom".into()),
    }
}

/// `argv` with `flag` set to `value`: the value in place (in either `--flag v` or `--flag=v` form),
/// else the pair at the end.
fn set_flag(argv: &mut Vec<String>, flag: &str, value: &str) {
    if let Some(i) = argv.iter().position(|a| a == flag) {
        match argv.get_mut(i + 1) {
            Some(v) => *v = value.to_owned(),
            None => argv.push(value.to_owned()),
        }
    } else if let Some(a) = argv.iter_mut().find(|a| a.strip_prefix(flag).is_some_and(|r| r.starts_with('='))) {
        *a = format!("{flag}={value}");
    } else {
        argv.extend([flag.to_owned(), value.to_owned()]);
    }
}

/// The profile after a pick of `value` for `field`; a missing profile starts from the suggestion.
fn apply(p: Option<&Profile>, field: Field, value: &str) -> Profile {
    let mut p = p.cloned().unwrap_or_else(suggested);
    match field {
        Field::Cli => {
            if !is_claude(&p) {
                p.argv = suggested().argv;
            }
        }
        _ => set_flag(&mut p.argv, field.flag(), value),
    }
    p
}

/// The argv as one line; an argument with a space is quoted.
fn argv_line(argv: &[String]) -> String {
    argv.iter().map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() }).collect::<Vec<_>>().join(" ")
}

/// The Roles state: the config as read, the open select and the last error.
#[derive(Default)]
pub struct Roles {
    cfg: Config,
    open: Option<(usize, Field)>,
    error: Option<String>,
    flow_error: Option<String>,
    /// Why `agents.json` did not load. The config shown is then the defaults, so no edit is saved
    /// until the file is fixed and the tab opens again.
    load_error: Option<String>,
}

impl Roles {
    /// Read `agents.json` again (the tab opened).
    pub fn load() -> Roles {
        let mut r = Roles::default();
        if let Some(path) = crate::paths::agents_config() {
            match Config::load(&path) {
                Ok(c) => r.cfg = c,
                Err(e) => {
                    r.error = Some(e.clone());
                    r.flow_error = Some(e.clone());
                    r.load_error = Some(e);
                }
            }
        }
        r
    }

    /// Save the new profile of `role`; the shown config changes only when the save works.
    fn put(&mut self, role: usize, p: Profile) {
        if let Some(e) = &self.load_error {
            self.error = Some(e.clone());
            return;
        }
        let Some(path) = crate::paths::agents_config() else { return };
        let mut next = self.cfg.clone();
        next.profiles.insert(ROLES[role].to_owned(), p);
        match next.save(&path) {
            Ok(()) => {
                self.cfg = next;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    pub fn flow(&self) -> &Flow {
        &self.cfg.flow
    }

    pub fn flow_error(&self) -> Option<&str> {
        self.flow_error.as_deref()
    }

    /// Save `flow` (C8); the shown flow changes only when the save works.
    pub fn set_flow(&mut self, flow: Flow) {
        if let Some(e) = &self.load_error {
            self.flow_error = Some(e.clone());
            return;
        }
        let Some(path) = crate::paths::agents_config() else { return };
        let mut next = self.cfg.clone();
        next.flow = flow;
        match next.save(&path) {
            Ok(()) => {
                self.cfg = next;
                self.flow_error = None;
            }
            Err(e) => self.flow_error = Some(e),
        }
    }

    fn pick(&mut self, role: usize, field: Field, value: &str) {
        self.open = None;
        let p = apply(self.cfg.profiles.get(ROLES[role]), field, value);
        self.put(role, p);
    }
}

/// The tab's body. The selects' lists go to `popups`, which the dialog puts above its card.
pub fn build(k: &Kit, s: &App, popups: &mut Vec<El>) -> El {
    let c = &k.c;
    let mut body = col().w_full().px(20.0).pt(10.0).pb(12.0).gap(10.0);
    body = body.child(text("Roles").font(k.ty.small_medium).color(c.text_faint));
    for (i, name) in ROLES.iter().enumerate() {
        let profile = s.roles.cfg.profiles.get(*name);
        let mut selects = row().gap(6.0).center_y();
        for f in Field::ALL {
            let open = s.roles.open == Some((i, f));
            let key = format!("role/{name}/{}", f.key());
            let sel = Select::new(key.clone(), f.options())
                .appearance(SelectAppearance::Field)
                .width(SELECT_W)
                .label(f.label())
                .current(shown(profile, f).as_deref())
                .placeholder(if profile.is_some() { "default" } else { "not set" })
                .open(open)
                .on_toggle(move |s: &mut App| s.roles.open = if open { None } else { Some((i, f)) })
                .on_close(|s: &mut App| s.roles.open = None)
                .on_choose(move |s: &mut App, v: &str, _tx| s.roles.pick(i, f, v))
                .build(k, popups);
            selects = selects.child(el().key(key).child(sel));
        }
        let head = row()
            .w_full()
            .center_y()
            .child(text((*name).to_owned()).font(k.ty.ui).color(c.text_base).w(80.0))
            .child(spacer())
            .child(selects);
        let argv_key = format!("role/{name}/argv");
        let mut line = row().w_full().gap(8.0).center_y();
        match profile {
            Some(p) => {
                line = line.child(text(argv_line(&p.argv)).key(argv_key).font(k.ty.mono).color(c.text_muted).wrap().grow(1.0));
            }
            None => {
                let hint = format!("not set \u{b7} suggested: {}", argv_line(&suggested().argv));
                let set = Button::new("Set")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .on_press(move |s: &mut App| s.roles.put(i, suggested()))
                    .build(k)
                    .key(format!("role/{name}/set"));
                line = line.child(text(hint).key(argv_key).font(k.ty.mono).color(c.text_faint).wrap().grow(1.0)).child(set);
            }
        }
        body = body.child(col().w_full().gap(4.0).child(head).child(line));
    }
    if let Some(e) = &s.roles.error {
        body = body.child(text(e.clone()).key("roles/error").font(k.ty.text_small).color(c.danger.fg).wrap());
    }
    body.child(crate::flow_section::build(k, s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(argv: &[&str]) -> Profile {
        Profile { argv: argv.iter().map(|a| a.to_string()).collect(), agent: String::new() }
    }

    #[test]
    fn selects_show_the_profile_or_custom() {
        let cases: [(Option<Profile>, [Option<&str>; 3]); 6] = [
            (None, [None, None, None]),
            (Some(Config::default().profiles["builder"].clone()), [Some(CLAUDE), Some("sonnet"), Some("medium")]),
            (Some(p(&["claude", "--model=haiku"])), [Some(CLAUDE), Some("haiku"), None]),
            (Some(p(&["claude", "--model", "claude-opus-4-1", "--effort", "xhigh"])), [Some(CLAUDE), Some("Custom"), Some("xhigh")]),
            (Some(p(&["codex"])), [Some("Custom"), Some("Custom"), Some("Custom")]),
            (Some(p(&["claude", "--effort"])), [Some(CLAUDE), None, None]),
        ];
        for (profile, want) in cases {
            let got = Field::ALL.map(|f| shown(profile.as_ref(), f));
            assert_eq!(got.each_ref().map(|g| g.as_deref()), want, "{profile:?}");
        }
    }

    #[test]
    fn a_pick_sets_one_flag_and_keeps_the_rest() {
        let base = p(&["claude", "--model", "sonnet", "--permission-mode", "auto", "--verbose"]);
        let cases = [
            (Some(&base), Field::Model, "opus", "claude --model opus --permission-mode auto --verbose"),
            (Some(&base), Field::Effort, "high", "claude --model sonnet --permission-mode auto --verbose --effort high"),
            (Some(&p(&["claude", "--effort=low"])), Field::Effort, "max", "claude --effort=max"),
            (Some(&p(&["claude", "--model"])), Field::Model, "haiku", "claude --model haiku"),
            (None, Field::Effort, "low", "claude --model opus --effort low --permission-mode auto"),
            (Some(&p(&["codex", "--x"])), Field::Cli, CLAUDE, "claude --model opus --effort medium --permission-mode auto"),
            (Some(&base), Field::Cli, CLAUDE, "claude --model sonnet --permission-mode auto --verbose"),
        ];
        for (profile, field, value, want) in cases {
            assert_eq!(argv_line(&apply(profile, field, value).argv), want, "{field:?} {value}");
        }
    }

    #[test]
    fn the_argv_line_quotes_spaces() {
        assert_eq!(argv_line(&p(&["claude", "--append-system-prompt", "be brief"]).argv), "claude --append-system-prompt \"be brief\"");
    }
}

//! The questions notch and card (H9). A small notch hangs from the top centre of the terminal panel
//! and counts what waits for the owner: blocked agents, open questions and open to-dos of the
//! selected project. Pressing it (or ⌘J) drops a card with one tab per item. A question shows its
//! options (the recommended one first), an input for a typed answer and a Send button; a to-do has
//! Done; a blocked agent shows the menu read from its screen (detail, question, numbered choices;
//! a digit or a click sends the choice) and a button that opens its terminal.
//!
//! Elements: `notch` (and `notch/label`), `qcard`, `qcard/tabs`, `qcard/item/<ID>`,
//! `qcard/option/<ID>/<i>` (`i` counts the options as shown, the recommended one is 0),
//! `qcard/input`, `qcard/send`, `qcard/done/<ID>`, `qcard/open/<address>`, `qcard/error`, `qcard/prompt/<address>` (with `/detail` and `/question`),
//! `qcard/choice/<address>/<n>` (`n` from 1).
//!
//! Answering writes `owner.json` first, then tells the question's coordinator terminal
//! (`Owner answered D7: <answer>`). A failed send leaves the answer saved and shows the error.

use rune::{Cursor, El, Key, KeyEvent, TextEdit, Tx, col, row, spacer, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, Kit, Placement, SegmentedControl, TextInput, popover,
};

use crate::App;
use crate::agents::Worker;
use crate::hc::model::AgentState;
use crate::hc::owner::{Question, Todo};
use crate::hc::{HcUpdate, watch};
use crate::prompt::{self, Prompt};
use crate::theme::{NOTCH_HEIGHT, NOTCH_OPEN_WIDTH};

/// How many terminal lines the card reads to find a blocked agent's menu.
const PROMPT_LINES: u32 = 40;

/// Shown when the menu changed between the owner looking and choosing; nothing was sent.
const PROMPT_CHANGED: &str = "The prompt changed; look again.";

/// Longest tab label before it is cut.
const TAB_MAX_CHARS: usize = 16;

/// One thing that waits for the owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Blocked {
        address: String,
        name: String,
        msg: String,
    },
    Question(Question),
    Todo(Todo),
}

impl Item {
    /// The id the keys use: an agent's address, `D7` or `T4`.
    pub fn id(&self) -> &str {
        match self {
            Item::Blocked { address, .. } => address,
            Item::Question(q) => &q.id,
            Item::Todo(t) => &t.id,
        }
    }

    fn tab_label(&self) -> String {
        let label = match self {
            Item::Blocked { name, .. } => name.as_str(),
            Item::Question(q) => &q.id,
            Item::Todo(t) => &t.id,
        };
        label.chars().take(TAB_MAX_CHARS).collect()
    }
}

/// The notch's label: "2 questions", "1 question", or "N items" when a to-do or a blocked agent is
/// counted. `None` when nothing is open (the notch is hidden).
pub fn label(blocked: usize, questions: usize, todos: usize) -> Option<String> {
    let total = blocked + questions + todos;
    let noun = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    match total {
        0 => None,
        _ if blocked + todos > 0 => Some(noun(total, "item", "items")),
        _ => Some(noun(total, "question", "questions")),
    }
}

/// The card's state.
#[derive(Default)]
pub struct Card {
    pub open: bool,
    /// The shown tab (clamped to the items).
    pub tab: usize,
    /// The option picked for the shown question (its text).
    pub pick: Option<String>,
    pub edit: TextEdit,
    pub error: Option<String>,
    /// The menu read from the shown blocked agent's screen (A2).
    pub menu: Option<Menu>,
    /// A choice is on its way to the agent.
    pub busy: bool,
}

/// What the card knows of a blocked agent's screen.
#[derive(Debug)]
pub struct Menu {
    pub address: String,
    /// The agent's turn and message when the screen was read; a new stamp reads it again.
    pub stamp: String,
    /// `None` while unread or when the screen has no form we know.
    pub prompt: Option<Prompt>,
}

impl Card {
    fn reset_answer(&mut self) {
        self.pick = None;
        self.edit = TextEdit::new("");
    }
}

/// What waits for the owner in the selected project: blocked agents, then questions, then to-dos.
pub fn items(s: &App) -> Vec<Item> {
    let Some(project) = s.projects.get(s.selected_project) else {
        return Vec::new();
    };
    let blocked = s
        .agents
        .iter()
        .filter(|a| a.project == Some(s.selected_project))
        .filter_map(|a| match &a.state {
            AgentState::NeedsYou { msg, .. } => Some(Item::Blocked {
                address: a.address.clone(),
                name: a.name.clone(),
                msg: msg.clone(),
            }),
            _ => None,
        });
    let questions = s
        .owner_state
        .open_questions(&project.name)
        .into_iter()
        .map(|q| Item::Question(q.clone()));
    let todos = s
        .owner_state
        .open_todos(&project.name)
        .into_iter()
        .map(|t| Item::Todo(t.clone()));
    blocked.chain(questions).chain(todos).collect()
}

/// The options of `q` in the order shown: the recommended one first.
pub fn shown_options(q: &Question) -> Vec<&str> {
    let first = q.recommended.filter(|i| *i < q.options.len());
    let rest = q
        .options
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != first)
        .map(|(_, o)| o.as_str());
    first
        .map(|i| q.options[i].as_str())
        .into_iter()
        .chain(rest)
        .collect()
}

fn counts(items: &[Item]) -> (usize, usize, usize) {
    let n = |f: fn(&Item) -> bool| items.iter().filter(|i| f(i)).count();
    (
        n(|i| matches!(i, Item::Blocked { .. })),
        n(|i| matches!(i, Item::Question(_))),
        n(|i| matches!(i, Item::Todo(_))),
    )
}

impl App {
    /// Read `owner.json` again when it changed (a stat); `force` reads it anyway.
    pub fn reload_owner(&mut self, force: bool) -> bool {
        let Some(source) = self.owner.clone() else {
            return false;
        };
        let stamp = source.stamp();
        if !force && stamp == self.owner_stamp {
            return false;
        }
        self.owner_stamp = stamp;
        match source.read() {
            Ok(state) => {
                let changed = state != self.owner_state;
                self.owner_state = state;
                changed
            }
            Err(e) => {
                self.card.error = Some(format!("couldn't read the owner's file: {e}"));
                true
            }
        }
    }

    /// ⌘J and a press on the notch: show or hide the card.
    pub fn toggle_card(&mut self) {
        if self.card.open {
            self.close_card();
        } else if !items(self).is_empty() {
            self.card = Card {
                open: true,
                ..Card::default()
            };
            self.sync_menu();
        }
    }

    pub fn close_card(&mut self) {
        self.card.open = false;
        self.card.error = None;
        self.card.menu = None;
        self.card.reset_answer();
    }

    fn shown_item(&self) -> Option<Item> {
        let items = items(self);
        let i = self.card.tab.min(items.len().saturating_sub(1));
        items.into_iter().nth(i)
    }

    fn pick_tab(&mut self, i: usize) {
        self.card.tab = i;
        self.card.error = None;
        self.card.reset_answer();
        self.sync_menu();
    }

    /// The blocked agent `address` as the core lists it now.
    fn blocked_worker(&self, address: &str) -> Option<Worker> {
        let t = self.hc_terminals.iter().find(|t| t.address == address)?;
        Some(Worker::from(t)).filter(|w| w.status == "blocked")
    }

    /// The menu on `screen` of blocked agent `address`.
    fn menu_of(&self, address: &str, screen: &str) -> Option<Prompt> {
        let w = self.blocked_worker(address)?;
        prompt::parse(&w.command, &w.kind, screen)
    }

    /// Read the shown blocked agent's screen when its turn or message is not the one last read
    /// (the agent's `status` and `turn` events reach here through the terminal list). The old menu
    /// stays until the new read lands.
    pub fn sync_menu(&mut self) {
        if !self.card.open {
            return;
        }
        let Some(Item::Blocked { address, .. }) = self.shown_item() else {
            return;
        };
        let Some(w) = self.blocked_worker(&address) else {
            return;
        };
        let stamp = format!("{}|{}", w.turn, w.msg);
        if self
            .card
            .menu
            .as_ref()
            .is_some_and(|m| m.address == address && m.stamp == stamp)
        {
            return;
        }
        let Some(dialer) = self.hc_dialer.clone() else {
            return;
        };
        let kept = self
            .card
            .menu
            .take()
            .filter(|m| m.address == address)
            .and_then(|m| m.prompt);
        self.card.menu = Some(Menu {
            address: address.clone(),
            stamp: stamp.clone(),
            prompt: kept,
        });
        self.run_hc(move || HcUpdate::Menu {
            screen: watch::read_screen(&dialer, &address, PROMPT_LINES),
            address,
            stamp,
        });
    }

    /// A digit key or a click on choice `n` (from 0) of the shown blocked agent's menu. A choice
    /// that asks for typed text opens the agent's terminal instead. Otherwise the screen is read
    /// again and the key goes only when the menu is the one the owner saw.
    pub fn choose(&mut self, n: usize) {
        let Some(Item::Blocked { address, .. }) = self.shown_item() else {
            return;
        };
        let Some(seen) = self
            .card
            .menu
            .as_ref()
            .filter(|m| m.address == address)
            .and_then(|m| m.prompt.clone())
        else {
            return;
        };
        let Some(choice) = seen.choices.get(n) else {
            return;
        };
        if choice.text {
            self.open_blocked(&address);
            return;
        }
        let (Some(w), Some(dialer)) = (self.blocked_worker(&address), self.hc_dialer.clone())
        else {
            return;
        };
        if self.card.busy {
            return;
        }
        self.card.busy = true;
        self.card.error = None;
        self.run_hc(move || {
            let result = watch::read_screen(&dialer, &address, PROMPT_LINES).and_then(|screen| {
                if prompt::parse(&w.command, &w.kind, &screen).as_ref() != Some(&seen) {
                    return Ok(Some(screen));
                }
                watch::press_keys(&dialer, &address, &[(n + 1).to_string()]).map(|()| None)
            });
            HcUpdate::Chose { address, result }
        });
    }

    /// Run `job` off the UI thread and deliver its update; a headless run cannot wait for a
    /// thread, so it runs at once.
    fn run_hc(&mut self, job: impl FnOnce() -> HcUpdate + Send + 'static) {
        if rune::is_headless() {
            let update = job();
            self.card_update(update);
            return;
        }
        let (tx, waker) = (self.hc_tx.clone(), self.waker.clone());
        std::thread::spawn(move || {
            let _ = tx.send(job());
            if let Some(w) = &waker {
                w();
            }
        });
    }

    /// A read or a choice finished.
    pub fn card_update(&mut self, update: HcUpdate) {
        match update {
            HcUpdate::Menu {
                address,
                stamp,
                screen,
            } => {
                let current = self
                    .card
                    .menu
                    .as_ref()
                    .is_some_and(|m| m.address == address && m.stamp == stamp);
                if !current {
                    return;
                }
                match screen {
                    Ok(screen) => self.set_prompt(&address, &screen),
                    Err(e) => self.card.error = Some(e),
                }
            }
            HcUpdate::Chose { address, result } => {
                self.card.busy = false;
                let shown = matches!(self.shown_item(), Some(Item::Blocked { address: a, .. }) if a == address);
                if !shown {
                    return;
                }
                match result {
                    Ok(None) => {}
                    Ok(Some(screen)) => {
                        self.card.error = Some(PROMPT_CHANGED.to_owned());
                        self.set_prompt(&address, &screen);
                    }
                    Err(e) => self.card.error = Some(e),
                }
            }
            _ => {}
        }
    }

    fn set_prompt(&mut self, address: &str, screen: &str) {
        let prompt = self.menu_of(address, screen);
        if let Some(m) = &mut self.card.menu {
            m.prompt = prompt;
        }
    }

    /// Pick the `n`-th shown option (0 is the first) of the shown question.
    pub fn pick_option(&mut self, n: usize) {
        let Some(Item::Question(q)) = self.shown_item() else {
            return;
        };
        if let Some(option) = shown_options(&q).get(n) {
            self.card.pick = Some((*option).to_owned());
            self.card.edit = TextEdit::new("");
        }
    }

    /// Answer the shown question with the typed text, else the picked option.
    pub fn send_answer(&mut self) {
        let Some(Item::Question(q)) = self.shown_item() else {
            return;
        };
        let typed = self.card.edit.text.trim().to_owned();
        let Some(answer) = (!typed.is_empty())
            .then_some(typed)
            .or_else(|| self.card.pick.clone())
        else {
            return;
        };
        let Some(source) = self.owner.clone() else {
            return;
        };
        self.card.error = None;
        match source.answer(&q.id, &answer) {
            Ok(coordinator) => {
                self.card.reset_answer();
                self.reload_owner(true);
                self.tell(coordinator, format!("Owner answered {}: {answer}\n", q.id));
            }
            Err(e) => self.card.error = Some(e.to_string()),
        }
    }

    /// Mark the to-do done.
    pub fn finish_todo(&mut self, id: &str) {
        let Some(todo) = self.owner_state.todos.iter().find(|t| t.id == id).cloned() else {
            return;
        };
        let Some(source) = self.owner.clone() else {
            return;
        };
        self.card.error = None;
        match source.done(id) {
            Ok(coordinator) => {
                self.reload_owner(true);
                self.tell(coordinator, format!("Owner did {id}: {}\n", todo.text));
            }
            Err(e) => self.card.error = Some(e.to_string()),
        }
    }

    /// The card's button for a blocked agent: show its terminal.
    pub fn open_blocked(&mut self, address: &str) {
        self.close_card();
        self.select_agent(address.to_owned());
    }

    /// Tell a coordinator terminal what the owner did. The answer is saved by now: a failure only
    /// shows on the card. A headless run cannot wait for a thread, so it sends at once.
    fn tell(&mut self, coordinator: Option<String>, text: String) {
        let Some(coordinator) = coordinator else {
            return;
        };
        let Some(dialer) = self.hc_dialer.clone() else {
            self.card.error = Some("answer saved; coordinator not reached: hc is off".to_owned());
            return;
        };
        if rune::is_headless() {
            self.told(watch::send_text(&dialer, &coordinator, &text));
            return;
        }
        let (tx, waker) = (self.hc_tx.clone(), self.waker.clone());
        std::thread::spawn(move || {
            let _ = tx.send(HcUpdate::Told(watch::send_text(
                &dialer,
                &coordinator,
                &text,
            )));
            if let Some(w) = &waker {
                w();
            }
        });
    }

    /// The result of telling a coordinator.
    pub fn told(&mut self, result: Result<(), String>) {
        if let Err(e) = result {
            self.card.error = Some(format!("answer saved; coordinator not reached: {e}"));
        }
    }
}

/// Keys of the card while it is open, from the input and from the window. True when used.
pub fn key(s: &mut App, ev: &KeyEvent) -> bool {
    if !s.card.open {
        return false;
    }
    let typing = s.card.edit.focused;
    match &ev.key {
        Key::Escape => s.close_card(),
        Key::Enter => s.send_answer(),
        Key::Text(t) if !typing && !ev.mods.cmd && !ev.mods.ctrl => {
            match t.parse::<usize>().ok().filter(|d| (1..=9).contains(d)) {
                Some(d) => {
                    s.pick_option(d - 1);
                    s.choose(d - 1);
                }
                None => return false,
            }
        }
        _ => return false,
    }
    true
}

/// The notch, pinned to the top centre of the terminal panel; `None` with nothing open.
pub fn notch(k: &Kit, s: &App) -> Option<El> {
    let items = items(s);
    let (blocked, questions, todos) = counts(&items);
    // A card that is open stays anchored to the notch while it shows an error.
    let text = label(blocked, questions, todos)
        .or_else(|| (s.card.open && s.card.error.is_some()).then(|| "all answered".to_owned()))?;
    let c = &k.c;
    let mut pill = row()
        .key("notch")
        .role("button")
        .label(text.clone())
        .h(NOTCH_HEIGHT)
        .px(14.0)
        .center_y()
        .radius(k.radius.lg)
        .fill(c.layer_01)
        .border(1.0, c.border_muted)
        .cursor(Cursor::Pointer)
        .on_press(|s: &mut App| s.toggle_card())
        .child(
            rune::text(text)
                .key("notch/label")
                .font(k.ty.text_small_medium)
                .color(c.text_base),
        );
    for shadow in k.shadow_layers(&k.elevation.floating) {
        pill = pill.box_shadow(shadow);
    }
    Some(
        row()
            .pin(Some(0.0), Some(0.0), Some(0.0), None)
            .justify_center()
            .child(pill),
    )
}

/// Push the card onto the overlay `layer` when it is open (and the notch it hangs from is shown).
pub fn card(k: &Kit, s: &App, layer: &mut Vec<El>) {
    if !s.card.open {
        return;
    }
    let items = items(s);
    if items.is_empty() && s.card.error.is_none() {
        return;
    }
    let c = &k.c;
    let tab = s.card.tab.min(items.len().saturating_sub(1));
    let mut body = col().w_full().gap(10.0).px(16.0).pt(14.0).pb(12.0);
    if items.len() > 1 {
        let labels: Vec<String> = items.iter().map(Item::tab_label).collect();
        let tabs = SegmentedControl::new(&labels, Some(tab))
            .label("Items")
            .fit()
            .on_pick(|s: &mut App, i, _tx| s.pick_tab(i))
            .build(k)
            .key("qcard/tabs");
        body = body.child(tabs);
    }
    if let Some(item) = items.get(tab) {
        body = body.child(item_view(k, s, item));
    }
    if let Some(error) = &s.card.error {
        body = body.child(
            text(error.clone())
                .key("qcard/error")
                .role("alert")
                .font(k.ty.text_small)
                .color(c.danger.fg)
                .w_full(),
        );
    }
    let hint = {
        let options = match items.get(tab) {
            Some(Item::Question(q)) => q.options.len().min(9),
            Some(Item::Blocked { address, .. }) => s
                .card
                .menu
                .as_ref()
                .filter(|m| &m.address == address)
                .and_then(|m| m.prompt.as_ref())
                .map_or(0, |p| p.choices.len()),
            _ => 0,
        };
        let verb = if matches!(items.get(tab), Some(Item::Blocked { .. })) {
            "send"
        } else {
            "pick"
        };
        let pick = match options {
            0 => String::new(),
            1 => format!("1 {verb}  "),
            n => format!("1-{n} {verb}  "),
        };
        format!("{pick}\u{23ce} next  esc close")
    };
    let footer = row()
        .w_full()
        .px(16.0)
        .pb(12.0)
        .child(
            text(hint)
                .key("qcard/keys")
                .font(k.ty.text_xsmall)
                .color(c.text_faint),
        )
        .child(spacer());
    let content = col()
        .key("qcard")
        .role("dialog")
        .label("Questions")
        .w(NOTCH_OPEN_WIDTH)
        .child(body)
        .child(footer);
    let surface = popover::raised(k, content, crate::theme::RADIUS_12);
    layer.push(rune_kit::scrim("qcard-scrim", |s: &mut App| s.close_card()));
    layer.push(popover::anchored(
        k,
        surface,
        "notch",
        Placement::Bottom,
        6.0,
    ));
}

/// A blocked agent's menu: the detail, the question and a button per choice.
fn menu_view(k: &Kit, address: &str, p: &Prompt) -> El {
    let c = &k.c;
    let mut view = col()
        .key(format!("qcard/prompt/{address}"))
        .w_full()
        .gap(10.0);
    if !p.detail.is_empty() {
        view = view.child(
            text(p.detail.join("\n"))
                .key(format!("qcard/prompt/{address}/detail"))
                .font(k.ty.text_small)
                .color(c.text_faint)
                .w_full(),
        );
    }
    if !p.question.is_empty() {
        view = view.child(
            text(p.question.clone())
                .key(format!("qcard/prompt/{address}/question"))
                .font(k.ty.ui)
                .color(c.text_strong)
                .w_full(),
        );
    }
    for (i, choice) in p.choices.iter().enumerate() {
        let (number, label) = (i + 1, choice.label.clone());
        view = view.child(
            Button::new(&choice.label)
                .variant(ButtonVariant::Outline)
                .label_with(move |font, ink| {
                    row()
                        .gap(8.0)
                        .center_y()
                        .w_full()
                        .child(text(number.to_string()).font(font).color(ink))
                        .child(text(label).font(font).color(ink))
                })
                .on_press(move |s: &mut App| s.choose(i))
                .build(k)
                .w_full()
                .key(format!("qcard/choice/{address}/{number}")),
        );
    }
    view
}

fn item_view(k: &Kit, s: &App, item: &Item) -> El {
    let c = &k.c;
    let heading = |meta: String| {
        text(meta)
            .font(k.ty.text_small)
            .color(c.text_faint)
            .w_full()
    };
    let body_text = |t: &str| {
        text(t.to_owned())
            .font(k.ty.ui)
            .color(c.text_strong)
            .w_full()
    };
    let mut view = col()
        .key(format!("qcard/item/{}", item.id()))
        .w_full()
        .gap(10.0);
    match item {
        Item::Blocked { address, name, msg } => {
            view = view.child(heading(format!("{name} is blocked")));
            let menu = s
                .card
                .menu
                .as_ref()
                .filter(|m| &m.address == address)
                .and_then(|m| m.prompt.as_ref());
            view = match menu {
                Some(p) => view.child(menu_view(k, address, p)),
                None => view.child(body_text(if msg.is_empty() {
                    "It waits for you."
                } else {
                    msg
                })),
            };
            let address = address.clone();
            view = view.child(
                row().child(
                    Button::new("Open terminal")
                        .variant(ButtonVariant::Contrast)
                        .on_press(move |s: &mut App| s.open_blocked(&address))
                        .build(k)
                        .key(format!("qcard/open/{}", item.id())),
                ),
            );
        }
        Item::Question(q) => {
            let meta = [
                Some(q.id.as_str()),
                (!q.from.is_empty()).then_some(q.from.as_str()),
                (!q.title.is_empty()).then_some(q.title.as_str()),
            ];
            view = view
                .child(heading(
                    meta.into_iter().flatten().collect::<Vec<_>>().join(" · "),
                ))
                .child(body_text(&q.question));
            let recommended = q.recommended.and_then(|i| q.options.get(i));
            for (i, option) in shown_options(q).into_iter().enumerate() {
                let picked = s.card.pick.as_deref() == Some(option);
                let marked = i == 0 && recommended.is_some();
                let (number, text_) = (i + 1, option.to_owned());
                let button = Button::new(option)
                    .variant(if picked {
                        ButtonVariant::Contrast
                    } else {
                        ButtonVariant::Outline
                    })
                    .label_with(move |font, ink| {
                        let mut line = row()
                            .gap(8.0)
                            .center_y()
                            .w_full()
                            .child(text(number.to_string()).font(font).color(ink))
                            .child(text(text_).font(font).color(ink));
                        if marked {
                            line = line
                                .child(spacer())
                                .child(text("recommended").font(font).color(ink).opacity(0.7));
                        }
                        line
                    })
                    .on_press(move |s: &mut App| s.pick_option(i))
                    .build(k)
                    .w_full()
                    .key(format!("qcard/option/{}/{i}", q.id));
                view = view.child(button);
            }
            let input = TextInput::new(&s.card.edit)
                .placeholder("Or type an answer")
                .label("Answer")
                .on_edit(|s: &mut App| &mut s.card.edit)
                .on_change(|s: &mut App, _tx| s.card.pick = None)
                .on_key(|s: &mut App, ev: &KeyEvent, _tx: &mut Tx| key(s, ev))
                .build(k)
                .key("qcard/input")
                .grow(1.0);
            let ready = s.card.pick.is_some() || !s.card.edit.text.trim().is_empty();
            let send = Button::new("Send")
                .variant(ButtonVariant::Submit)
                .control_size(ControlSize::Regular)
                .disabled(!ready)
                .on_press(|s: &mut App| s.send_answer())
                .build(k)
                .key("qcard/send");
            view = view.child(row().w_full().gap(8.0).center_y().child(input).child(send));
        }
        Item::Todo(t) => {
            let kind = if t.kind == "know" { "to know" } else { "to do" };
            let from = if t.from.is_empty() {
                String::new()
            } else {
                format!(" · from {}", t.from)
            };
            let id = t.id.clone();
            view = view
                .child(heading(format!("{} · {kind}{from}", t.id)))
                .child(body_text(&t.text))
                .child(
                    row().child(
                        Button::new("Done")
                            .variant(ButtonVariant::Contrast)
                            .on_press(move |s: &mut App| s.finish_todo(&id))
                            .build(k)
                            .key(format!("qcard/done/{}", t.id)),
                    ),
                );
        }
    }
    view
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_counts_questions_or_items() {
        assert_eq!(label(0, 0, 0), None);
        assert_eq!(label(0, 1, 0).as_deref(), Some("1 question"));
        assert_eq!(label(0, 2, 0).as_deref(), Some("2 questions"));
        assert_eq!(
            label(1, 0, 0).as_deref(),
            Some("1 item"),
            "a blocked agent makes it items"
        );
        assert_eq!(label(0, 0, 1).as_deref(), Some("1 item"), "so does a to-do");
        assert_eq!(label(1, 2, 3).as_deref(), Some("6 items"));
    }

    #[test]
    fn the_recommended_option_is_shown_first() {
        let q = |options: &[&str], recommended| Question {
            options: options.iter().map(|o| (*o).to_owned()).collect(),
            recommended,
            ..Question::default()
        };
        assert_eq!(
            shown_options(&q(&["a", "b", "c"], Some(2))),
            ["c", "a", "b"]
        );
        assert_eq!(shown_options(&q(&["a", "b"], None)), ["a", "b"]);
        assert_eq!(
            shown_options(&q(&["a", "b"], Some(9))),
            ["a", "b"],
            "an index out of range is ignored"
        );
        assert!(shown_options(&q(&[], Some(0))).is_empty());
    }
}

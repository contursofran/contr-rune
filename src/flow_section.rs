//! Settings → Agents → Flow (C8, Go D187): how the orchestrator takes an issue to a merge. It
//! shows and edits the `flow` key of `agents.json` through `agents_config` (unknown keys kept):
//! three presets, the row of steps the flow makes, then Plan first, Review, Debugger after,
//! Workers at once and Merge. Every change saves at once; a failed save shows in `flow/error`.
//!
//! Keys: `flow-presets` (`/segment<i>`), `flow/step/<name>` and `/detail`, `flow-plan`,
//! `flow-review`, `flow/review/note`, `flow-merge` (each `/segment<i>`),
//! `flow/debugger_after` and `flow/workers` (each with `/value`, `/minus`, `/plus`), `flow/error`.
//!
//! Differs from Go: the steppers are two small buttons (rune-kit has no Stepper).

use rune::{El, col, row, spacer, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, SegmentedControl};

use crate::App;
use crate::agents_config::Flow;

/// The most a stepper counts to; the least is 1.
const MAX_COUNT: i64 = 99;

/// The presets: label, `plan`, `review`.
const PRESETS: [(&str, &str, &str); 3] = [
    ("Builder only", "never", "none"),
    ("Build and review", "when-needed", "code"),
    ("Plan, build, review", "always", "code"),
];
const PLAN: [(&str, &str); 3] = [("Never", "never"), ("When needed", "when-needed"), ("Always", "always")];
const REVIEW: [(&str, &str); 3] = [("None", "none"), ("Code PRs", "code"), ("Every PR", "every")];
const MERGE: [(&str, &str); 2] = [("Ask me", "ask"), ("When CI and review pass", "auto")];

/// The preset whose `plan` and `review` match the flow; none for a flow that matches no preset.
fn preset_of(f: &Flow) -> Option<usize> {
    PRESETS.iter().position(|(_, plan, review)| f.plan == *plan && f.review == *review)
}

/// The flow after a pick of preset `i`: `plan` and `review` change, the rest stays.
fn with_preset(f: &Flow, i: usize) -> Flow {
    let (_, plan, review) = PRESETS[i];
    Flow { plan: plan.into(), review: review.into(), ..f.clone() }
}

/// `n` moved by `delta`, kept within 1 and `MAX_COUNT`.
fn bump(n: i64, delta: i64) -> i64 {
    (n + delta).clamp(1, MAX_COUNT)
}

/// The steps of the flow: name, detail and whether the step is off.
fn steps(f: &Flow) -> [(&'static str, String, bool); 5] {
    let planner = match f.plan.as_str() {
        "never" => ("off", true),
        "always" => ("always", false),
        _ => ("when needed", false),
    };
    let reviewer = match f.review.as_str() {
        "none" => ("off", true),
        "every" => ("every PR", false),
        _ => ("code PRs", false),
    };
    let merge = if f.merge == "auto" { "when CI and review pass" } else { "after your OK" };
    [
        ("Planner", planner.0.into(), planner.1),
        ("Builder", "builds".into(), false),
        ("Reviewer", reviewer.0.into(), reviewer.1),
        ("Fix rounds", format!("debugger after {}", f.debugger_after), false),
        ("Merge", merge.into(), false),
    ]
}

/// Save the flow `change` makes from the shown one.
fn edit(s: &mut App, change: impl FnOnce(&mut Flow)) {
    let mut f = s.roles.flow().clone();
    change(&mut f);
    s.roles.set_flow(f);
}

/// A segmented control over `options` with the one matching `current` chosen.
fn choice(k: &Kit, key: &str, options: &[(&str, &'static str)], current: &str, set: fn(&mut Flow, &'static str)) -> El {
    let labels: Vec<&str> = options.iter().map(|(l, _)| *l).collect();
    let values: Vec<&'static str> = options.iter().map(|(_, v)| *v).collect();
    SegmentedControl::new(&labels, options.iter().position(|(_, v)| *v == current))
        .label(key.to_owned())
        .fit()
        .on_pick(move |s: &mut App, i, _tx| edit(s, |f| set(f, values[i])))
        .build(k)
        .key(key.to_owned())
}

/// The number, its unit and the two buttons that change it.
fn stepper(k: &Kit, key: &str, n: i64, unit: &str, set: fn(&mut Flow, i64)) -> El {
    let c = &k.c;
    let button = |name: &'static str, glyph: &'static str, delta: i64| {
        Button::new(glyph)
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Small)
            .on_press(move |s: &mut App| edit(s, |f| set(f, delta)))
            .build(k)
            .key(format!("{key}/{name}"))
    };
    row()
        .gap(6.0)
        .center_y()
        .child(text(n.to_string()).key(format!("{key}/value")).font(k.ty.mono).color(c.text_strong))
        .child(text(unit.to_owned()).font(k.ty.text_small).color(c.text_faint))
        .child(button("minus", "\u{2212}", -1))
        .child(button("plus", "+", 1))
}

/// The Flow section for the Agents tab.
pub fn build(k: &Kit, s: &App) -> El {
    let c = &k.c;
    let f = s.roles.flow();
    let labels = PRESETS.map(|(l, ..)| l);
    let presets = SegmentedControl::new(&labels, preset_of(f))
        .label("Flow")
        .width(None)
        .on_pick(|s: &mut App, i, _tx| {
            let f = with_preset(s.roles.flow(), i);
            s.roles.set_flow(f);
        })
        .build(k)
        .key("flow-presets");

    let mut flow_steps = row().w_full().gap(6.0).center_y().px(8.0).py(8.0).radius(6.0).fill(c.layer_01);
    for (i, (name, detail, off)) in steps(f).into_iter().enumerate() {
        if i > 0 {
            flow_steps = flow_steps.child(text("\u{2192}").font(k.ty.text_small).color(c.text_faint));
        }
        let (head, sub) = if off { (c.text_faint, c.text_faint) } else { (c.text_base, c.text_muted) };
        flow_steps = flow_steps.child(
            col()
                .key(format!("flow/step/{name}"))
                .grow(1.0)
                .child(text(name).font(k.ty.small_medium).color(head))
                .child(text(detail).key(format!("flow/step/{name}/detail")).font(k.ty.text_small).color(sub)),
        );
    }

    let line = |label: &str, control: El| {
        row().w_full().center_y().child(text(label.to_owned()).font(k.ty.ui).color(c.text_base)).child(spacer()).child(control)
    };
    let mut body = col()
        .w_full()
        .pt(8.0)
        .gap(10.0)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(text("Flow").font(k.ty.small_medium).color(c.text_strong))
                .child(text("How the orchestrator takes an issue to a merge").font(k.ty.text_small).color(c.text_faint)),
        )
        .child(presets)
        .child(flow_steps)
        .child(line("Plan first", choice(k, "flow-plan", &PLAN, &f.plan, |f, v| f.plan = v.into())))
        .child(line("Review", choice(k, "flow-review", &REVIEW, &f.review, |f, v| f.review = v.into())));
    if f.review == "code" {
        body = body.child(
            text("docs-only PRs: the orchestrator reads the diff").key("flow/review/note").font(k.ty.text_small).color(c.text_faint),
        );
    }
    body = body
        .child(line(
            "Debugger after",
            stepper(k, "flow/debugger_after", f.debugger_after, "failed fix rounds", |f, d| f.debugger_after = bump(f.debugger_after, d)),
        ))
        .child(line("Workers at once", stepper(k, "flow/workers", f.workers, "reviewers count", |f, d| f.workers = bump(f.workers, d))))
        .child(line("Merge", choice(k, "flow-merge", &MERGE, &f.merge, |f, v| f.merge = v.into())));
    if let Some(e) = s.roles.flow_error() {
        body = body.child(text(e.to_owned()).key("flow/error").font(k.ty.text_small).color(c.danger.fg).wrap());
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow(plan: &str, review: &str) -> Flow {
        Flow { plan: plan.into(), review: review.into(), ..Flow::default() }
    }

    #[test]
    fn the_flow_chooses_a_preset_and_a_preset_sets_the_flow() {
        let cases = [
            (flow("never", "none"), Some(0)),
            (flow("when-needed", "code"), Some(1)),
            (flow("always", "code"), Some(2)),
            (flow("always", "every"), None),
            (flow("never", "code"), None),
        ];
        for (f, want) in cases {
            assert_eq!(preset_of(&f), want, "{f:?}");
        }
        let base = Flow { debugger_after: 5, workers: 7, merge: "auto".into(), ..Flow::default() };
        for i in 0..PRESETS.len() {
            let f = with_preset(&base, i);
            assert_eq!(preset_of(&f), Some(i));
            assert_eq!((f.debugger_after, f.workers, f.merge.as_str()), (5, 7, "auto"), "the other values stay");
            assert!(f.validate().is_ok());
        }
    }

    #[test]
    fn every_option_is_a_valid_flow_value() {
        for (_, v) in PLAN {
            assert!(Flow { plan: v.into(), ..Flow::default() }.validate().is_ok(), "{v}");
        }
        for (_, v) in REVIEW {
            assert!(Flow { review: v.into(), ..Flow::default() }.validate().is_ok(), "{v}");
        }
        for (_, v) in MERGE {
            assert!(Flow { merge: v.into(), ..Flow::default() }.validate().is_ok(), "{v}");
        }
    }

    #[test]
    fn the_steppers_stop_at_1_and_99() {
        assert_eq!([bump(1, -1), bump(2, -1), bump(98, 1), bump(99, 1), bump(150, 1)], [1, 1, 99, 99, 99]);
    }

    #[test]
    fn an_off_step_reads_off() {
        let off = steps(&flow("never", "none"));
        assert_eq!((off[0].1.as_str(), off[0].2, off[2].1.as_str(), off[2].2), ("off", true, "off", true));
        let on = steps(&Flow { merge: "auto".into(), ..flow("always", "every") });
        assert_eq!((on[0].1.as_str(), on[0].2, on[2].1.as_str(), on[4].1.as_str()), ("always", false, "every PR", "when CI and review pass"));
        assert_eq!(on[3].1, "debugger after 2");
    }
}

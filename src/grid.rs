//! The emulator's visible grid → one Rune text element per row, coloured with spans.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color as AColor, CursorShape, NamedColor, Rgb};
use rune::{Color, Span};
use rune_kit::Kit;

use crate::session::Events;
use crate::theme;

/// The default colours of the grid, taken from the kit; the ANSI palette below stays fixed.
#[derive(Clone, Copy)]
pub struct Ink {
    /// Default text and the focused cursor block.
    pub text: Color,
    /// Dim default text.
    pub dim: Color,
    /// The panel colour the grid sits on (ANSI colours are mixed with it).
    pub bg: Color,
    /// The unfocused cursor.
    pub selected: Color,
}

impl Ink {
    pub fn from_kit(k: &Kit) -> Ink {
        Ink { text: k.c.text_base, dim: k.c.text_muted, bg: k.c.bg_base, selected: k.c.pressed }
    }
}

/// One drawn row: its text and the colour spans over its bytes.
pub struct Line {
    pub text: String,
    pub spans: Vec<Span>,
}

/// The 16 ANSI colours (a muted palette that sits on contr's #121212).
const ANSI: [u32; 16] = [
    0x1c1c1c, 0xf07178, 0xa6d189, 0xe5c07b, 0x82aaff, 0xc792ea, 0x89ddff, 0xbdbdbd,
    0x5a5a5a, 0xff8b92, 0xc3e88d, 0xffcb6b, 0x9cc4ff, 0xddb3f5, 0xa3ebff, 0xededed,
];

use theme::{srgb as rgb, srgb8};

fn from_rgb(c: Rgb) -> Color { srgb8(c.r, c.g, c.b) }

fn indexed(i: u8) -> Color {
    match i {
        0..=15 => rgb(ANSI[i as usize]),
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            srgb8(level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            srgb8(v, v, v)
        }
    }
}

/// A cell colour, honouring colours the program set with OSC 4/10/11.
fn resolve(c: AColor, colors: &Colors, fg: bool, ink: &Ink) -> Color {
    match c {
        AColor::Spec(rgb) => from_rgb(rgb),
        AColor::Indexed(i) => colors[i as usize].map(from_rgb).unwrap_or_else(|| indexed(i)),
        AColor::Named(n) => {
            if let Some(set) = colors[n] { return from_rgb(set); }
            match n {
                NamedColor::Foreground | NamedColor::BrightForeground => ink.text,
                NamedColor::DimForeground => ink.dim,
                NamedColor::Background => ink.bg,
                NamedColor::Cursor => ink.text,
                n if (n as usize) < 16 => indexed(n as u8),
                // Dim variants: the normal colour, dimmed.
                n if (n as usize) >= NamedColor::DimBlack as usize => {
                    let base = n as usize - NamedColor::DimBlack as usize;
                    indexed(base as u8).mix(ink.bg, 0.35)
                }
                _ => if fg { ink.text } else { ink.bg },
            }
        }
    }
}

/// The visible rows, with the block cursor drawn as an inverted cell.
pub fn lines(term: &Term<Events>, focused: bool, ink: &Ink) -> Vec<Line> {
    let content = term.renderable_content();
    let rows = term.screen_lines();
    let colors = content.colors;
    let offset = content.display_offset as i32;
    let show_cursor = content.mode.contains(TermMode::SHOW_CURSOR) && content.cursor.shape != CursorShape::Hidden;
    let cursor = content.cursor.point;

    let mut out: Vec<Line> = (0..rows).map(|_| Line { text: String::new(), spans: Vec::new() }).collect();
    // Trailing default-coloured blanks are trimmed per row; `keep` is the byte length to keep.
    let mut keep = vec![0usize; rows];

    for cell in content.display_iter {
        let row = cell.point.line.0 + offset;
        if row < 0 || row as usize >= rows { continue; }
        let r = row as usize;
        if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) { continue; }

        let line = &mut out[r];
        let start = line.text.len();
        let ch = if cell.flags.contains(Flags::HIDDEN) { ' ' } else { cell.c };
        line.text.push(if ch == '\0' { ' ' } else { ch });
        if let Some(extra) = cell.zerowidth() { for z in extra { line.text.push(*z); } }
        let end = line.text.len();

        let (mut fg, mut bg) = (resolve(cell.fg, colors, true, ink), resolve(cell.bg, colors, false, ink));
        let mut has_bg = !matches!(cell.bg, AColor::Named(NamedColor::Background));
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
            has_bg = true;
        }
        if cell.flags.contains(Flags::DIM) { fg = fg.mix(ink.bg, 0.4); }
        let is_cursor = show_cursor && cell.point == cursor;
        if is_cursor {
            if focused {
                bg = ink.text;
                fg = ink.bg;
                has_bg = true;
            } else {
                // Unfocused window: hollow-ish cursor, as terminals draw it.
                bg = ink.selected;
                has_bg = true;
            }
        }

        let default_fg = fg == ink.text;
        let plain = default_fg && !has_bg && !cell.flags.intersects(Flags::BOLD | Flags::ITALIC | Flags::ALL_UNDERLINES | Flags::STRIKEOUT);
        if !(plain && ch == ' ') { keep[r] = end; }
        if plain { continue; }

        let mut span = Span::new(start, end);
        if !default_fg { span = span.color(fg); }
        if has_bg { span = span.bg(bg, 0.0); }
        if cell.flags.contains(Flags::BOLD) { span = span.bold(); }
        if cell.flags.contains(Flags::ITALIC) { span = span.italic(); }
        if cell.flags.intersects(Flags::ALL_UNDERLINES) { span = span.underline(fg); }

        // Merge with the previous span when it continues it with the same look.
        if let Some(prev) = line.spans.last_mut()
            && prev.end as usize == start
            && (Span { start: prev.start, end: prev.end, ..span.clone() }) == *prev
        {
            prev.end = end as u32;
        } else {
            line.spans.push(span);
        }
    }

    for (line, k) in out.iter_mut().zip(keep) {
        line.text.truncate(k);
        line.spans.retain(|s| (s.start as usize) < k);
        if let Some(last) = line.spans.last_mut() { last.end = last.end.min(k as u32); }
    }
    out
}

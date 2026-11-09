//! The shared tab set of a project (T4, D190): every Claude session in the sidebar's order, then the
//! selected worker, then the plain terminals. The set is the same whichever tab is selected.

/// What a tab shows, for the order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind<'a> {
    /// A Claude session (an orchestrator, a foreign session, a plain tab running Claude), with its
    /// place in the sidebar (`usize::MAX` when it has no row).
    Claude(usize),
    /// A worker, by address.
    Worker(&'a str),
    /// A plain terminal.
    Plain,
}

/// The indexes of `kinds` to show, in order: Claude sessions by sidebar place (equal places keep
/// their order), then the worker `keep` (if there is one), then the plain terminals in their order.
/// Every other worker is left out: the caller detaches it.
pub fn order(kinds: &[Kind], keep: Option<&str>) -> Vec<usize> {
    let mut claude: Vec<usize> = (0..kinds.len()).filter(|i| matches!(kinds[*i], Kind::Claude(_))).collect();
    claude.sort_by_key(|i| match kinds[*i] {
        Kind::Claude(rank) => rank,
        _ => usize::MAX,
    });
    let worker = (0..kinds.len()).filter(|i| matches!(kinds[*i], Kind::Worker(a) if Some(a) == keep));
    let plain = (0..kinds.len()).filter(|i| kinds[*i] == Kind::Plain);
    claude.into_iter().chain(worker).chain(plain).collect()
}

#[cfg(test)]
mod tests {
    use super::Kind::*;
    use super::*;

    #[test]
    fn the_order_of_the_tab_set() {
        // (tabs in the strip as they are, the worker kept, the order that shows)
        let table: &[(&[Kind], Option<&str>, &[usize])] = &[
            (&[], None, &[]),
            // Claude first, by sidebar place; plain terminals last, in order.
            (&[Plain, Claude(2), Plain, Claude(0)], None, &[3, 1, 0, 2]),
            // The kept worker sits between the Claude sessions and the plain terminals.
            (&[Plain, Worker("t17"), Claude(0)], Some("t17"), &[2, 1, 0]),
            // Any other worker is out; with no kept worker all of them are.
            (&[Worker("t17"), Worker("t18"), Claude(0)], Some("t18"), &[2, 1]),
            (&[Worker("t17"), Plain], None, &[1]),
            // A Claude tab without a row follows the ones with a row; equal places keep their order.
            (&[Claude(usize::MAX), Claude(1), Claude(usize::MAX)], None, &[1, 0, 2]),
        ];
        for (kinds, keep, want) in table {
            assert_eq!(order(kinds, *keep), *want, "{kinds:?} keeping {keep:?}");
        }
    }
}

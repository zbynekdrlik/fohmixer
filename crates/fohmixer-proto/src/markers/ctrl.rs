//! A frame control's Tuner (#104): a `stage`, `alert` or `param_toggle` that
//! names a `+CTRL` id takes the tagged track by index, and an id on two
//! Tuners of a Live set, or on none, binds nothing.

use std::collections::BTreeMap;

use super::{Entry, TagProblem, each_control_mut};
use crate::layout::{Anchor, Binding, Control, CtrlId, Layout};
use crate::markers::CtrlReport;

/// Who carries a `+CTRL` id (#104): the entries, per Live set and id.
pub(super) type Ctrls = BTreeMap<(String, CtrlId), Vec<usize>>;

/// Every `+CTRL` id of every Live set and its entries.
pub(super) fn carriers(entries: &[Entry<'_>]) -> Ctrls {
    let mut ctrls = Ctrls::new();
    for (i, entry) in entries.iter().enumerate() {
        for id in &entry.marker.ctrls {
            ctrls
                .entry((entry.found.instance.clone(), *id))
                .or_default()
                .push(i);
        }
    }
    ctrls
}

/// An id on two Tuners of a set is a problem on each (never a guess, #104).
pub(super) fn mark_duplicates(entries: &mut [Entry<'_>], ctrls: &Ctrls) {
    for ((_, id), list) in ctrls {
        if list.len() > 1 {
            for i in list {
                entries[*i]
                    .problems
                    .push(TagProblem::DuplicateCtrl { id: *id });
            }
        }
    }
}

/// The track each id binds: only an id on exactly one Tuner has one.
pub(super) fn tracks(entries: &[Entry<'_>], ctrls: &Ctrls) -> BTreeMap<(String, CtrlId), Anchor> {
    ctrls
        .iter()
        .filter_map(|(key, list)| match list.as_slice() {
            [one] => Some((key.clone(), entries[*one].track().anchor)),
            _ => None,
        })
        .collect()
}

/// Every id the frame names and how many Tuners carry it (#104).
pub(super) fn reports(frame: &Layout, ctrls: &Ctrls) -> Vec<CtrlReport> {
    frame
        .ctrls()
        .into_iter()
        .map(|(instance, id)| CtrlReport {
            tuners: ctrls.get(&(instance.clone(), id)).map_or(0, Vec::len),
            instance,
            id,
        })
        .collect()
}

/// A binding that names a `+CTRL` id takes its track by index, its instance
/// and path staying; with no track it keeps the id (#104).
fn fill_binding(binding: &mut Binding, tracks: &BTreeMap<(String, CtrlId), Anchor>) {
    if let Anchor::Ctrl { id } = binding.anchor
        && let Some(anchor) = tracks.get(&(binding.instance.clone(), id))
    {
        binding.anchor = anchor.clone();
    }
}

/// Fills every `stage`, `alert` and `param_toggle` that names a `+CTRL` id
/// (#104; a view's copied rail too, so it runs after the views). A control
/// that still names its track stays as it is.
pub(super) fn fill(layout: &mut Layout, tracks: &BTreeMap<(String, CtrlId), Anchor>) {
    each_control_mut(layout, &mut |control: &mut Control| match control {
        Control::Stage { binding, .. } | Control::Alert { binding, .. } => {
            fill_binding(binding, tracks);
        }
        Control::ParamToggle { targets, .. } => {
            for target in targets {
                fill_binding(&mut target.binding, tracks);
            }
        }
        _ => {}
    });
}

//! `STREAM LUFS` in the control column (#184, the approved mockup
//! `docs/mockups/mbc-layout-v3.html`): a `lufs_readout` of the frame's
//! `global`, read only. Its reading is #151's, unchanged: the hub's LUFS
//! item of the readout's point (`Layout::lufs_points`), shown through
//! `behave::lufs::look` with the readout's target (the number with one
//! decimal, `−∞` in silence, green only within ±1.0 LU of the target).
//! Its caption is the readout's label, under the number its target line;
//! a problem (no LUFS meter on the track) takes that line's place. It
//! takes no touch.

use fohmixer_proto::layout::{Binding, LufsSpec};
use leptos::prelude::*;

use crate::behave::lufs::{look, target_text, track_key};
use crate::store::LiveStore;

/// The readout `label` of `binding`'s track against `target`; `compact`
/// draws its number 30 px (the FOH column), else 40 px (the MBC page).
#[component]
pub fn StreamLufs(
    label: String,
    target: i8,
    binding: Binding,
    #[prop(into)] compact: Signal<bool>,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let key = StoredValue::new(track_key(&binding));
    let spec = Some(LufsSpec {
        target: Some(target),
    });
    let shown = Memo::new(move |_| {
        let point = key
            .try_with_value(|k| {
                k.as_ref().and_then(|(instance, target)| {
                    store.lufs.try_with(|l| l.point(instance, target))
                })
            })
            .flatten()
            .unwrap_or_default();
        look(point.tenths, point.problem, spec)
    });
    let state = move || shown.try_with(|l| l.state.as_str()).unwrap_or("none");
    let number = move || shown.try_with(|l| l.text.clone()).flatten();
    let problem = move || shown.try_with(|l| l.problem).flatten();
    let goal = target_text(target);
    let line = move || problem().map_or_else(|| goal.clone(), str::to_string);
    let size = move || {
        if compact.try_get().unwrap_or(false) {
            "compact"
        } else {
            "full"
        }
    };
    view! {
        <div class="slufs" data-testid="stream-lufs-box" data-size=size aria-hidden="true">
            <div class="slufs-cap" data-testid="stream-lufs-cap">{label}</div>
            <div class="slufs-n" data-testid="stream-lufs" data-l=state>
                {number}
            </div>
            <div class="slufs-goal" data-testid="stream-lufs-goal" data-problem=problem>
                {line}
            </div>
        </div>
    }
}

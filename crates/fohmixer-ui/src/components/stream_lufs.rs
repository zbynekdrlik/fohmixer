//! `STREAM LUFS` in the control column (#184, the approved mockup
//! `docs/mockups/mbc-layout-v3.html`): a `lufs_readout` of the frame's
//! `global`, read only. Its reading is #151's, unchanged: the hub's LUFS
//! item of the readout's point (`Layout::lufs_points`), shown through
//! `behave::lufs::look` with the readout's target (the number with one
//! decimal, `−∞` in silence, green only within ±1.0 LU of the target).
//! Its caption is the readout's label, under the number its target line;
//! a problem (no LUFS meter on the track) takes that line's place. It
//! takes no touch.

use fohmixer_proto::layout::{Binding, Control, Layout, LufsSpec, readouts};
use fohmixer_proto::sectors::{SectorId, SectorLayout, SectorsDoc};
use leptos::prelude::*;

use crate::behave::lufs::{look, stream_page, stream_shown, target_text, track_key};
use crate::binding::sectors::{Picks, picked};
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

/// The global `lufs_readout`s (#184, `STREAM LUFS`) under the sectors
/// page's ARRANGE (`pages::sectors`): each while
/// `behave::lufs::stream_shown` says so (always on a page of one
/// Live set's faders, the MBC hub's, at 40 px; on the FOH page only while a
/// picked layout shows its Live set's faders, at 30 px).
pub(crate) fn stream_readouts(
    layout: &Layout,
    doc: Memo<Option<SectorsDoc>>,
    picks: RwSignal<Picks>,
) -> impl IntoView + use<> {
    let one = stream_page(layout);
    readouts(layout)
        .into_iter()
        .filter_map(|control| match control {
            Control::LufsReadout {
                label,
                target,
                binding,
            } => Some((label.clone(), *target, binding.clone())),
            _ => None,
        })
        .map(|(label, target, binding)| {
            let instance = binding.instance.clone();
            let shown = Memo::new(move |_| {
                doc.try_with(|doc| {
                    let picked: Vec<&SectorLayout> = doc
                        .as_ref()
                        .map(|doc| {
                            picks
                                .try_with(|picks| {
                                    SectorId::ALL
                                        .into_iter()
                                        .filter_map(|sector| picked(doc, picks, sector))
                                        .collect()
                                })
                                .unwrap_or_default()
                        })
                        .unwrap_or_default();
                    stream_shown(&instance, &picked, one)
                })
                .unwrap_or(false)
            });
            let compact = Signal::derive(move || !one);
            move || {
                shown.try_get().unwrap_or(false).then(|| {
                    view! {
                        <StreamLufs
                            label=label.clone()
                            target=target
                            binding=binding.clone()
                            compact=compact
                        />
                    }
                })
            }
        })
        .collect_view()
}

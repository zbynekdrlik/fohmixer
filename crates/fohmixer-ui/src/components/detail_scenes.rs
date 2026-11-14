//! The channel detail's SCENES (#207; the owner's approved mockup
//! `docs/mockups/scenes-v3.html`, frame 1): a section of `detail-right`
//! after LAYOUTS and before MUTE ALARM, shown only by a hub that takes
//! scenes (its `scenes` document came). Only the scenes this strip has a
//! rule for: a row each, its name, the MUTE chip (— / MUTE / UNMUTE) and ✕,
//! then the dB (− value +, `= FADER`: the fader's dB now) and the glide's
//! time (− value +); `+ ADD SCENE` opens a sheet of the scenes heard that
//! the strip has no row for (one tap adds a row at the fader's dB, 2 s). The
//! scene that came last is marked (a green left edge). Every key asks the
//! hub (`LiveStore::set_scene_rule`); the hub tells every page, a refusal
//! flashes the key (`data-refused`). The decisions are `behave::scenes`;
//! nothing here writes a Live key.

use fohmixer_proto::scenes::{SceneRule, ScenesDoc};
use fohmixer_proto::sectors::StripRef;
use leptos::prelude::*;

use super::{fail_flash, owns_touches};
use crate::behave::scenes::{
    addable, db_down, db_text, db_up, fader_db, is_last, mute_next, mute_word, new_rule, rows,
    time_down, time_text, time_up,
};
use crate::binding::sectors::detail_ref;
use crate::store::{LiveStore, Slot, on_refusal};

/// The fader's dB now as a rule takes it (none without a value).
fn fader_now(volume: Option<RwSignal<Slot>>) -> Option<f64> {
    fader_db(volume.and_then(|s| s.try_with(Slot::number).flatten()))
}

/// SCENES for the strip shown; nothing from a hub that takes no scene.
#[component]
pub fn DetailScenes(
    strip: fohmixer_proto::layout::Strip,
    volume: Option<RwSignal<Slot>>,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let id = StoredValue::new(detail_ref(&strip));
    let on = Memo::new(move |_| {
        store
            .scenes
            .try_with(|s| s.shown().is_some())
            .unwrap_or(false)
    });
    let sheet = RwSignal::new(false);
    move || {
        if !on.try_get().unwrap_or(false) {
            return None;
        }
        let named = id.try_with_value(Option::is_some).unwrap_or(false);
        Some(if named {
            view! { <ScenesSection strip_id=id volume=volume sheet=sheet /> }.into_any()
        } else {
            view! {
                <div class="scn-sec" data-testid="detail-scenes">
                    <h2 class="detail-title">"SCENES"</h2>
                    <div class="lay-note" data-testid="detail-scenes-none">
                        "This fader has no name yet, so it cannot keep scene rules."
                    </div>
                </div>
            }
            .into_any()
        })
    }
}

/// The rows of a strip with an identity, ADD SCENE and its sheet.
#[component]
fn ScenesSection(
    strip_id: StoredValue<Option<StripRef>>,
    volume: Option<RwSignal<Slot>>,
    sheet: RwSignal<bool>,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let doc = Memo::new(move |_| {
        store
            .scenes
            .try_with(|s| s.shown().cloned())
            .flatten()
            .unwrap_or_else(ScenesDoc::empty)
    });
    let rows_memo = Memo::new(move |_| {
        let Some(Some(strip)) = strip_id.try_get_value() else {
            return Vec::new();
        };
        doc.try_with(|d| rows(d, &strip)).unwrap_or_default()
    });
    let count = move || {
        let n = rows_memo.try_with(Vec::len).unwrap_or(0);
        if n == 0 { String::new() } else { n.to_string() }
    };
    let empty = move || rows_memo.try_with(Vec::is_empty).unwrap_or(true);
    // Keyed by the scene: a value's change keeps the row (and its refused
    // flash).
    let scenes = move || {
        rows_memo
            .try_get()
            .unwrap_or_default()
            .into_iter()
            .map(|(scene, _)| scene)
            .collect::<Vec<_>>()
    };
    let no_keys: Vec<String> = Vec::new();
    let open = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = sheet.try_set(true);
    };
    view! {
        <div class="scn-sec" data-testid="detail-scenes">
            <div class="col-head">
                <h2 class="detail-title">"SCENES"</h2>
                <span class="col-now" data-testid="scenes-count">
                    {count}
                </span>
            </div>
            <Show when=empty>
                <div class="scn-empty" data-testid="scenes-empty">
                    "No scene moves this fader."
                </div>
            </Show>
            <For each=scenes key=|scene| scene.clone() let:scene>
                <SceneRow strip_id=strip_id scene=scene rows=rows_memo volume=volume />
            </For>
            <button
                type="button"
                class="scn-add"
                use:owns_touches=no_keys
                data-testid="scene-add"
                on:pointerdown=open
            >
                "+ ADD SCENE"
            </button>
            <Show when=move || sheet.try_get().unwrap_or(false)>
                <SceneSheet strip_id=strip_id doc=doc volume=volume sheet=sheet />
            </Show>
        </div>
    }
}

/// One scene's row: its name, the mute chip, ✕, then the dB and the time.
#[component]
fn SceneRow(
    strip_id: StoredValue<Option<StripRef>>,
    scene: String,
    rows: Memo<Vec<(String, SceneRule)>>,
    volume: Option<RwSignal<Slot>>,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let name = StoredValue::new(scene.clone());
    let rule = Memo::new(move |_| {
        name.try_with_value(|n| {
            rows.try_with(|r| r.iter().find(|(s, _)| s == n).map(|(_, rule)| *rule))
                .flatten()
        })
        .flatten()
    });
    let last = move || {
        let last = store
            .scene
            .try_with(|s| s.as_ref().map(|n| n.name.clone()))
            .flatten();
        name.try_with_value(|n| is_last(last.as_deref(), n))
            .unwrap_or(false)
            .to_string()
    };
    let failed = RwSignal::new(false);
    // Each key asks the hub for the row's rule after `change` (none: the
    // row goes).
    let ask = move |change: &dyn Fn(SceneRule) -> Option<SceneRule>| {
        let (Some(Some(strip)), Some(scene), Some(Some(now))) = (
            strip_id.try_get_value(),
            name.try_get_value(),
            rule.try_get(),
        ) else {
            return;
        };
        store.set_scene_rule(
            &strip,
            &scene,
            change(now),
            Some(on_refusal(fail_flash(failed))),
        );
    };
    let mute = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        ask(&|r| {
            Some(SceneRule {
                db: r.db,
                mute: mute_next(r.mute, r.db.is_some()),
                time_ms: r.time_ms,
            })
        });
    };
    let remove = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        ask(&|_| None);
    };
    let down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        ask(&|r| {
            Some(SceneRule {
                db: Some(db_down(r.db)),
                mute: r.mute,
                time_ms: r.time_ms,
            })
        });
    };
    let up = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        ask(&|r| {
            Some(SceneRule {
                db: Some(db_up(r.db)),
                mute: r.mute,
                time_ms: r.time_ms,
            })
        });
    };
    let take = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let now = fader_now(volume);
        ask(&|r| {
            Some(SceneRule {
                db: now.or(r.db),
                mute: r.mute,
                time_ms: r.time_ms,
            })
        });
    };
    let slower = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        ask(&|r| {
            Some(SceneRule {
                db: r.db,
                mute: r.mute,
                time_ms: time_up(r.time_ms),
            })
        });
    };
    let faster = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        ask(&|r| {
            Some(SceneRule {
                db: r.db,
                mute: r.mute,
                time_ms: time_down(r.time_ms),
            })
        });
    };
    let mute_text = move || mute_word(rule.try_get().flatten().and_then(|r| r.mute));
    let mute_state = move || {
        rule.try_get()
            .flatten()
            .and_then(|r| r.mute)
            .map_or("none", |m| if m.value() { "mute" } else { "unmute" })
    };
    let db = move || db_text(rule.try_get().flatten().and_then(|r| r.db));
    let time = move || {
        rule.try_get()
            .flatten()
            .map(|r| time_text(r.time_ms))
            .unwrap_or_default()
    };
    let refused = move || failed.try_get().unwrap_or(false).to_string();
    let scene_attr = scene.clone();
    let k1: Vec<String> = Vec::new();
    let k2: Vec<String> = Vec::new();
    let k3: Vec<String> = Vec::new();
    let k4: Vec<String> = Vec::new();
    let k5: Vec<String> = Vec::new();
    let k6: Vec<String> = Vec::new();
    let k7: Vec<String> = Vec::new();
    view! {
        <div
            class="scn-item"
            data-testid="scene-row"
            data-scene=scene_attr
            data-last=last
            data-refused=refused
        >
            <div class="scn-line">
                <span class="scn-name" data-testid="scene-name">
                    {scene}
                </span>
                <button
                    type="button"
                    class="scn-btn scn-mute"
                    use:owns_touches=k1
                    data-testid="scene-mute"
                    data-mute=mute_state
                    on:pointerdown=mute
                >
                    {mute_text}
                </button>
                <button
                    type="button"
                    class="scn-x"
                    use:owns_touches=k2
                    data-testid="scene-remove"
                    on:pointerdown=remove
                >
                    "✕"
                </button>
            </div>
            <div class="scn-edit">
                <button
                    type="button"
                    class="scn-btn scn-sm"
                    use:owns_touches=k3
                    data-testid="scene-db-down"
                    on:pointerdown=down
                >
                    "−"
                </button>
                <span class="scn-num" data-testid="scene-db">
                    {db}
                </span>
                <button
                    type="button"
                    class="scn-btn scn-sm"
                    use:owns_touches=k4
                    data-testid="scene-db-up"
                    on:pointerdown=up
                >
                    "+"
                </button>
                <button
                    type="button"
                    class="scn-btn scn-take"
                    use:owns_touches=k5
                    data-testid="scene-take"
                    on:pointerdown=take
                >
                    "= FADER"
                </button>
                <span class="scn-sep"></span>
                <button
                    type="button"
                    class="scn-btn scn-sm"
                    use:owns_touches=k6
                    data-testid="scene-time-down"
                    on:pointerdown=faster
                >
                    "−"
                </button>
                <span class="scn-num scn-t" data-testid="scene-time">
                    {time}
                </span>
                <button
                    type="button"
                    class="scn-btn scn-sm"
                    use:owns_touches=k7
                    data-testid="scene-time-up"
                    on:pointerdown=slower
                >
                    "+"
                </button>
            </div>
        </div>
    }
}

/// ADD SCENE's sheet: the scenes heard without a row; a tap adds one at
/// the fader's dB, 2 s, and closes it; ✕ closes it.
#[component]
fn SceneSheet(
    strip_id: StoredValue<Option<StripRef>>,
    doc: Memo<ScenesDoc>,
    volume: Option<RwSignal<Slot>>,
    sheet: RwSignal<bool>,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let picks = Memo::new(move |_| {
        let Some(Some(strip)) = strip_id.try_get_value() else {
            return Vec::new();
        };
        doc.try_with(|d| addable(d, &strip)).unwrap_or_default()
    });
    let none_heard = move || doc.try_with(|d| d.known.is_empty()).unwrap_or(true);
    let all_have = move || !none_heard() && picks.try_with(Vec::is_empty).unwrap_or(true);
    let last = move || {
        store
            .scene
            .try_with(|s| s.as_ref().map(|n| n.name.clone()))
            .flatten()
    };
    let list = move || {
        picks
            .try_get()
            .unwrap_or_default()
            .into_iter()
            .map(|scene| {
                let failed = RwSignal::new(false);
                let now_mark = is_last(last().as_deref(), &scene).then_some("NOW");
                let pick_name = StoredValue::new(scene.clone());
                let pick = move |ev: web_sys::PointerEvent| {
                    ev.prevent_default();
                    let (Some(Some(strip)), Some(scene)) =
                        (strip_id.try_get_value(), pick_name.try_get_value())
                    else {
                        return;
                    };
                    let rule = new_rule(fader_now(volume));
                    if store.set_scene_rule(
                        &strip,
                        &scene,
                        Some(rule),
                        Some(on_refusal(fail_flash(failed))),
                    ) {
                        let _ = sheet.try_set(false);
                    }
                };
                let keys: Vec<String> = Vec::new();
                let pick_attr = scene.clone();
                view! {
                    <button
                        type="button"
                        class="scn-pick"
                        use:owns_touches=keys
                        data-testid="scene-pick"
                        data-scene=pick_attr
                        data-refused=move || failed.try_get().unwrap_or(false).to_string()
                        on:pointerdown=pick
                    >
                        {scene}
                        {now_mark.map(|m| view! { <small>{m}</small> })}
                    </button>
                }
            })
            .collect_view()
    };
    let close = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = sheet.try_set(false);
    };
    let keys: Vec<String> = Vec::new();
    view! {
        <div class="scn-sheet" data-testid="scene-sheet">
            <div class="scn-panel">
                <div class="scn-sheet-head">
                    <b>"ADD SCENE"</b>
                    <button
                        type="button"
                        class="scn-x"
                        use:owns_touches=keys
                        data-testid="scene-sheet-close"
                        on:pointerdown=close
                    >
                        "✕"
                    </button>
                </div>
                <Show when=none_heard>
                    <div class="scn-empty">"No scene has come yet."</div>
                </Show>
                <Show when=all_have>
                    <div class="scn-empty">"Every scene has a row."</div>
                </Show>
                <div class="scn-picks">{list}</div>
            </div>
        </div>
    }
}

//! One shared `requestAnimationFrame` loop for every moving part (S4 design
//! note §2; iemmixer's meter registry @ 22372bc, generalised): fader caps,
//! pan dots, meter bars, status pills and the TechAlert blink register a
//! frame function; a value change never re-renders the view. The loop runs
//! while anything is registered.
//!
//! A frame function may register or unregister others (a component that
//! mounts or unmounts during a frame): the registry is taken out for the
//! frame and merged back afterwards ([`Registry`], unit-tested).
//!
//! Every frame's gap to the one before also goes to the page's frame count
//! (`diag::frame`, #5 K4: the `perf` report's frame rate and longest
//! frame): one more reader of this loop, not a second loop. A loop that
//! starts again counts its first gap from its start, not from the last
//! frame before it stopped.
//!
//! An idle page (#51, [`set_idle`]) draws at most one frame per
//! [`IDLE_FRAME_MS`]: each next frame waits a timer, then asks
//! `requestAnimationFrame` (drawing stays on the display's frames), and says
//! it was paced (`diag::frame`: no frame rate counts it, and it is a long
//! frame only for what lies beyond the pace). The wake clears a pending
//! timer and asks for a frame at once; the store's stop puts the loop back
//! to full rate. One chain of frames exists at any time: a timer, a frame
//! asked for or a frame running.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::behave::idle::IDLE_FRAME_MS;

/// A frame function: called with the frame's time and the time since the
/// previous frame (ms, at most 100).
pub type FrameFn = Box<dyn FnMut(f64, f64)>;

/// The longest frame step handed to the frame functions (a tab that was
/// hidden resumes without a jump). An idle page's frames come a little more
/// than [`IDLE_FRAME_MS`] apart (the timer, then the next display frame), so
/// its meters still fall at nearly their real speed; raise this cap with
/// that pace.
const MAX_STEP_MS: f64 = 100.0;

/// The registry of frame functions, by id.
pub struct Registry<T> {
    items: Vec<(usize, T)>,
    next: usize,
    /// Ids unregistered while the items were taken out for a frame.
    removed: Vec<usize>,
    taken: bool,
}

impl<T> Default for Registry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Registry<T> {
    pub const fn new() -> Self {
        Self {
            items: Vec::new(),
            next: 0,
            removed: Vec::new(),
            taken: false,
        }
    }

    /// Adds an item: its id.
    pub fn add(&mut self, item: T) -> usize {
        let id = self.next;
        self.next += 1;
        self.items.push((id, item));
        id
    }

    /// Removes the item `id` (also while the items are taken out).
    pub fn remove(&mut self, id: usize) {
        self.items.retain(|(i, _)| *i != id);
        if self.taken {
            self.removed.push(id);
        }
    }

    /// How many items are registered (outside a frame).
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Takes the items out for a frame.
    pub fn take(&mut self) -> Vec<(usize, T)> {
        self.taken = true;
        std::mem::take(&mut self.items)
    }

    /// Puts a frame's items back: those removed during the frame are
    /// dropped, those added during it follow the others.
    pub fn restore(&mut self, mut items: Vec<(usize, T)>) {
        let removed = std::mem::take(&mut self.removed);
        items.retain(|(id, _)| !removed.contains(id));
        items.append(&mut self.items);
        self.items = items;
        self.taken = false;
    }
}

thread_local! {
    static REGISTRY: RefCell<Registry<FrameFn>> = const { RefCell::new(Registry::new()) };
    static RUNNING: Cell<bool> = const { Cell::new(false) };
    static LAST: Cell<f64> = const { Cell::new(0.0) };
    /// The page is idle (#51): the next frame waits [`IDLE_FRAME_MS`].
    static IDLE: Cell<bool> = const { Cell::new(false) };
    /// An idle page's pending timer before its next frame.
    static TIMER: Cell<Option<TimeoutHandle>> = const { Cell::new(None) };
    /// The frame asked for came through that timer.
    static PACED: Cell<bool> = const { Cell::new(false) };
}

/// The page went idle (`on`) or woke (#51): idle, each next frame waits
/// [`IDLE_FRAME_MS`]; awake, a pending wait ends and the next frame is asked
/// for at once (a frame already asked for comes within one display frame).
pub fn set_idle(on: bool) {
    IDLE.set(on);
    if on {
        return;
    }
    if let Some(timer) = TIMER.take() {
        timer.clear();
        request();
    }
}

/// Registers a frame function and starts the loop if it is stopped: its id.
pub fn register(frame: FrameFn) -> usize {
    let id = REGISTRY.with_borrow_mut(|r| r.add(frame));
    if !RUNNING.get() {
        RUNNING.set(true);
        LAST.set(crate::dom::now());
        schedule();
    }
    id
}

/// A component's frame function: `make` builds it once `root` is in the
/// page (it gets the element), and it is unregistered with the component.
/// Every moving part (fader, pan, meter, status pill, TechAlert) animates
/// through this.
pub fn animate<F>(root: NodeRef<html::Div>, make: F)
where
    F: FnOnce(web_sys::HtmlDivElement) -> FrameFn + 'static,
{
    let frame_id = StoredValue::new(None::<usize>);
    root.on_load(move |el: web_sys::HtmlDivElement| {
        frame_id.set_value(Some(register(make(el))));
    });
    on_cleanup(move || {
        if let Some(Some(id)) = frame_id.try_get_value() {
            unregister(id);
        }
    });
}

/// Unregisters a frame function (the loop stops when none is left).
pub fn unregister(id: usize) {
    REGISTRY.with_borrow_mut(|r| r.remove(id));
}

/// The loop's next frame: at once, or after an idle page's pace.
fn schedule() {
    if !IDLE.get() {
        request();
        return;
    }
    let wait = Duration::from_millis(IDLE_FRAME_MS as u64);
    let paced = || {
        TIMER.set(None);
        PACED.set(true);
        request();
    };
    match set_timeout_with_handle(paced, wait) {
        Ok(timer) => TIMER.set(Some(timer)),
        Err(_) => RUNNING.set(false),
    }
}

/// Asks for the next frame (`requestAnimationFrame`).
fn request() {
    let callback = Closure::once_into_js(move |time: f64| {
        tick(time);
    });
    let requested = web_sys::window()
        .map(|w| w.request_animation_frame(callback.unchecked_ref()).is_ok())
        .unwrap_or(false);
    if !requested {
        RUNNING.set(false);
    }
}

/// One frame: the page's frame count, every registered function, then the
/// next frame while any is left.
fn tick(time: f64) {
    let paced = PACED.replace(false);
    let gap = time - LAST.get();
    let step = gap.clamp(0.0, MAX_STEP_MS);
    LAST.set(time);
    crate::diag::frame(gap, paced);
    let mut items = REGISTRY.with_borrow_mut(Registry::take);
    for (_, frame) in &mut items {
        frame(time, step);
    }
    let empty = REGISTRY.with_borrow_mut(|r| {
        r.restore(items);
        r.is_empty()
    });
    if empty {
        RUNNING.set(false);
    } else {
        schedule();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids<T>(r: &Registry<T>) -> Vec<usize> {
        r.items.iter().map(|(id, _)| *id).collect()
    }

    #[test]
    fn items_get_fresh_ids_and_leave_by_id() {
        let mut r: Registry<&str> = Registry::default();
        assert!(r.is_empty());
        assert_eq!(r.add("a"), 0);
        assert_eq!(r.add("b"), 1);
        assert_eq!(r.add("c"), 2);
        assert!(!r.is_empty());
        r.remove(1);
        assert_eq!(ids(&r), [0, 2]);
        assert_eq!(r.len(), 2);
        r.remove(7);
        assert_eq!(r.len(), 2);
        assert_eq!(r.add("d"), 3, "ids are never reused");
    }

    #[test]
    fn a_frame_may_add_and_remove_items() {
        let mut r = Registry::new();
        let a = r.add("a");
        let b = r.add("b");
        let taken = r.take();
        assert!(r.is_empty(), "taken out for the frame");
        // During the frame: `a` leaves, `c` joins, `c` stays.
        r.remove(a);
        let c = r.add("c");
        r.restore(taken);
        assert_eq!(ids(&r), [b, c]);
        // A removal outside a frame is not remembered for the next one.
        r.remove(b);
        let taken = r.take();
        r.restore(taken);
        assert_eq!(ids(&r), [c]);
        // One added and removed within the same frame is gone.
        let taken = r.take();
        let d = r.add("d");
        r.remove(d);
        r.restore(taken);
        assert_eq!(ids(&r), [c]);
    }
}

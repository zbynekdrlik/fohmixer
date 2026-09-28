//! One shared `requestAnimationFrame` loop for every moving part (S4 design
//! note §2; iemmixer's meter registry @ 22372bc, generalised): fader caps,
//! pan dots, meter bars, status pills and the TechAlert blink register a
//! frame function; a value change never re-renders the view. The loop runs
//! while anything is registered.
//!
//! A frame function may register or unregister others (a component that
//! mounts or unmounts during a frame): the registry is taken out for the
//! frame and merged back afterwards ([`Registry`], unit-tested).

use std::cell::{Cell, RefCell};

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

/// A frame function: called with the frame's time and the time since the
/// previous frame (ms, at most 100).
pub type FrameFn = Box<dyn FnMut(f64, f64)>;

/// The longest frame step handed to the frame functions (a tab that was
/// hidden resumes without a jump).
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
}

/// Registers a frame function and starts the loop if it is idle: its id.
pub fn register(frame: FrameFn) -> usize {
    let id = REGISTRY.with_borrow_mut(|r| r.add(frame));
    if !RUNNING.get() {
        RUNNING.set(true);
        LAST.set(crate::dom::now());
        schedule();
    }
    id
}

/// Unregisters a frame function (the loop stops when none is left).
pub fn unregister(id: usize) {
    REGISTRY.with_borrow_mut(|r| r.remove(id));
}

fn schedule() {
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

/// One frame: every registered function, then the next frame while any is
/// left.
fn tick(time: f64) {
    let step = (time - LAST.get()).clamp(0.0, MAX_STEP_MS);
    LAST.set(time);
    let mut items = REGISTRY.with_borrow_mut(Registry::take);
    for (_, frame) in &mut items {
        frame(time, step);
    }
    let idle = REGISTRY.with_borrow_mut(|r| {
        r.restore(items);
        r.is_empty()
    });
    if idle {
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

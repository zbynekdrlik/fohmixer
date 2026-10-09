//! The simulated window backend (#71 PR E): the hub's backend off Windows,
//! in the E2E harness and in the tests. Live's editor windows are numbers,
//! tied to Live's `is_editor_open` as far as the hub changes it: asking Live
//! to open an editor opens one ([`Backend::live_opened`], unless a test
//! turned that off), and the window closes only when Live closed the editor
//! ([`Backend::live_closed`]). A window handed back without that (a guard
//! that failed, a close check that left Live alone, the hub's stop) stays,
//! as the editor stays open in Live. An editor already open in Live opens no
//! new window; the hub reads `is_editor_open` first and refuses it. Its
//! picture is synthetic, [`SIM_WIDTH`] × [`SIM_HEIGHT`] like Pro-Q 4 at
//! 100 % (Pro-Q's own child window is there unless a test holds it back,
//! `child_late`: the take then refuses the window, as the hub's Windows
//! take does), a counter drawn in that steps every [`STEP_MS`] so frames
//! differ.
//!
//! Every take, touch, release and close is recorded, in order: in memory
//! for the tests ([`SimHandle::records`]) and, when the sim has a record
//! file (`<data>/eq-sim.jsonl`, one JSON object a line), there too, where the
//! E2E harness reads them (`GET /sim/eq`). A touch record carries its time
//! (`t`, ms since the sim started), so a test can measure the gaps between
//! a contact's injections. A test can make the sim refuse the editor's
//! points (`refuse`) as a window covering it would, or make each grab take
//! its time (`grab_delay`) as the PC's BitBlt of a 4.3 MB picture does.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use fohmixer_proto::eq::reason;

use super::{Backend, Phase, Pixels, Taken, WindowId};

/// The simulated picture's width (Pro-Q 4's at 100 %).
pub const SIM_WIDTH: u32 = 1349;
/// Its height.
pub const SIM_HEIGHT: u32 = 809;
/// How often the picture's counter steps (ms): 4 new frames a second.
pub const STEP_MS: f64 = 250.0;
/// The record file in the hub's data folder.
pub const RECORD_FILE: &str = "eq-sim.jsonl";
/// Why the sim refuses a point.
pub const REFUSED: &str = "the point is another window's";

/// The counter a picture shows `elapsed_ms` after the sim started.
pub fn counter(elapsed_ms: f64) -> u64 {
    (elapsed_ms / STEP_MS).floor() as u64
}

/// The synthetic picture of `count`: a dark field, a bar whose x moves with
/// the count, and the count's low bits as blocks in the top-left corner.
pub fn picture(count: u64, width: u32, height: u32) -> Pixels {
    let (w, h) = (width as usize, height as usize);
    let mut bgra = vec![0_u8; w * h * 4];
    for (i, pixel) in bgra.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (i % w, i / w);
        let bar = (count as usize * 37) % w.max(1);
        let lit = x.abs_diff(bar) < 6;
        let bit = (x < 16 * 8 && y < 16) && ((count >> (x / 16)) & 1) == 1;
        let shade: u8 = if lit || bit { 0xF0 } else { 0x20 };
        *pixel = [shade, shade / 2, shade, 0xFF];
    }
    Pixels {
        width,
        height,
        bgra,
    }
}

/// A simulated window.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SimWindow {
    pid: u32,
    topmost: bool,
}

#[derive(Debug)]
struct State {
    windows: BTreeMap<WindowId, SimWindow>,
    next: u64,
    /// Asking Live to open an editor opens a window.
    auto_open: bool,
    /// The editor's points are refused.
    refuse: bool,
    /// The picture never changes (its counter stays 0).
    still: bool,
    /// How long a grab takes.
    grab_delay: Duration,
    /// Pro-Q's own child window is not there yet (Live shows the editor's
    /// window before the plug-in attaches its view).
    child_late: bool,
    /// The last picture drawn and its count: a grab draws only a new count
    /// or size (drawing one costs the worker tens of ms in a test build).
    drawn: Option<(u64, Pixels)>,
    records: Vec<Value>,
    file: Option<PathBuf>,
    started: Instant,
}

impl State {
    fn add(&mut self, pid: u32) -> WindowId {
        self.next += 1;
        let window = WindowId(self.next);
        self.windows.insert(
            window,
            SimWindow {
                pid,
                topmost: false,
            },
        );
        window
    }

    fn record(&mut self, record: Value) {
        if let Some(path) = &self.file {
            let line = format!("{record}\n");
            let written = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut f| f.write_all(line.as_bytes()));
            if let Err(e) = written {
                tracing::warn!(error = %e, "the simulated window backend cannot write its record");
            }
        }
        self.records.push(record);
    }
}

/// The simulated backend (the worker owns it).
#[derive(Debug)]
pub struct Sim {
    state: Arc<Mutex<State>>,
}

/// A test's hold on a sim: its windows, its switches and its records.
#[derive(Debug, Clone)]
pub struct SimHandle {
    state: Arc<Mutex<State>>,
}

impl Sim {
    /// A sim with no window, recording to `file` when given.
    pub fn new(file: Option<PathBuf>) -> (Self, SimHandle) {
        let state = Arc::new(Mutex::new(State {
            windows: BTreeMap::new(),
            next: 0,
            auto_open: true,
            refuse: false,
            still: false,
            grab_delay: Duration::ZERO,
            child_late: false,
            drawn: None,
            records: Vec::new(),
            file,
            started: Instant::now(),
        }));
        (
            Self {
                state: Arc::clone(&state),
            },
            SimHandle { state },
        )
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl SimHandle {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A window of process `pid` appears.
    pub fn add_window(&self, pid: u32) -> WindowId {
        self.lock().add(pid)
    }

    /// The window goes away.
    pub fn remove_window(&self, window: WindowId) {
        self.lock().windows.remove(&window);
    }

    /// Whether asking Live to open an editor opens a window.
    pub fn auto_open(&self, on: bool) {
        self.lock().auto_open = on;
    }

    /// Whether the editor's points are refused.
    pub fn refuse(&self, on: bool) {
        self.lock().refuse = on;
    }

    /// Whether the picture stays the same.
    pub fn still(&self, on: bool) {
        self.lock().still = on;
    }

    /// How long each grab takes from now on (the worker waits for it).
    pub fn grab_delay(&self, delay: Duration) {
        self.lock().grab_delay = delay;
    }

    /// Whether Pro-Q's own child window is still missing from the editor
    /// windows (a take refuses them meanwhile).
    pub fn child_late(&self, on: bool) {
        self.lock().child_late = on;
    }

    /// Whether `window` is on top of every window.
    pub fn topmost(&self, window: WindowId) -> Option<bool> {
        self.lock().windows.get(&window).map(|w| w.topmost)
    }

    /// The windows there now.
    pub fn windows(&self) -> Vec<WindowId> {
        self.lock().windows.keys().copied().collect()
    }

    /// Everything recorded so far.
    pub fn records(&self) -> Vec<Value> {
        self.lock().records.clone()
    }
}

impl Backend for Sim {
    fn editors(&mut self) -> Result<Vec<WindowId>, String> {
        Ok(self.lock().windows.keys().copied().collect())
    }

    fn windows_of(&mut self, pid: u32) -> Result<Vec<WindowId>, String> {
        Ok(self
            .lock()
            .windows
            .iter()
            .filter(|(_, w)| w.pid == pid)
            .map(|(id, _)| *id)
            .collect())
    }

    fn live_opened(&mut self) {
        let mut state = self.lock();
        if state.auto_open {
            state.add(0);
        }
    }

    fn take(&mut self, window: WindowId) -> Result<Taken, String> {
        let mut state = self.lock();
        if state.child_late {
            return Err(reason::NO_PICTURE.to_string());
        }
        let found = state
            .windows
            .get_mut(&window)
            .ok_or_else(|| "no such window".to_string())?;
        let was_topmost = found.topmost;
        found.topmost = true;
        let pid = found.pid;
        state.record(json!({"op": "take", "window": window.0}));
        Ok(Taken {
            window,
            picture: window,
            pid,
            was_topmost,
            width: SIM_WIDTH,
            height: SIM_HEIGHT,
        })
    }

    fn alive(&mut self, taken: &Taken) -> bool {
        self.lock().windows.contains_key(&taken.window)
    }

    fn grab(&mut self, taken: &Taken) -> Result<Pixels, String> {
        // A slow grab waits without the lock (a test reads the records).
        let delay = self.lock().grab_delay;
        std::thread::sleep(delay);
        let state = self.lock();
        if !state.windows.contains_key(&taken.window) {
            return Err("no such window".to_string());
        }
        let count = if state.still {
            0
        } else {
            counter(state.started.elapsed().as_secs_f64() * 1000.0)
        };
        let drawn = state
            .drawn
            .as_ref()
            .filter(|(n, p)| *n == count && (p.width, p.height) == (taken.width, taken.height))
            .map(|(_, p)| p.clone());
        drop(state);
        if let Some(pixels) = drawn {
            return Ok(pixels);
        }
        let pixels = picture(count, taken.width, taken.height);
        self.lock().drawn = Some((count, pixels.clone()));
        Ok(pixels)
    }

    fn touch(&mut self, taken: &Taken, phase: Phase, at: (i32, i32)) -> Result<(), String> {
        let mut state = self.lock();
        if phase.checked() && state.refuse {
            return Err(REFUSED.to_string());
        }
        let t = state.started.elapsed().as_secs_f64() * 1000.0;
        state.record(json!({
            "op": "touch",
            "window": taken.window.0,
            "phase": phase.name(),
            "x": at.0,
            "y": at.1,
            "t": t,
        }));
        Ok(())
    }

    fn release(&mut self, taken: &Taken) {
        let mut state = self.lock();
        if let Some(window) = state.windows.get_mut(&taken.window) {
            window.topmost = taken.was_topmost;
        }
        state.record(json!({"op": "release", "window": taken.window.0}));
    }

    fn live_closed(&mut self, taken: &Taken) {
        // Live closes its editor's window once `is_editor_open` is false; a
        // host's window (the probe's) is no editor of Live's.
        if taken.pid == 0 {
            self.lock().windows.remove(&taken.window);
        }
    }

    fn close_window(&mut self, taken: &Taken) {
        let mut state = self.lock();
        state.windows.remove(&taken.window);
        state.record(json!({"op": "close", "window": taken.window.0}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_counter_steps_four_times_a_second() {
        assert_eq!(counter(0.0), 0);
        assert_eq!(counter(249.9), 0);
        assert_eq!(counter(250.0), 1);
        assert_eq!(counter(1000.0), 4);
        assert_eq!(STEP_MS, 250.0);
    }

    #[test]
    fn a_picture_is_its_size_and_shows_its_count() {
        let one = picture(1, 300, 20);
        assert_eq!((one.width, one.height), (300, 20));
        assert_eq!(one.bgra.len(), 300 * 20 * 4);
        // Count 1: the bar at x 37, the first bit block lit.
        let at = |p: &Pixels, x: usize, y: usize| p.bgra[(y * 300 + x) * 4];
        assert_eq!(at(&one, 37, 19), 0xF0);
        assert_eq!(at(&one, 32, 19), 0xF0, "5 px beside the bar is lit");
        assert_eq!(at(&one, 42, 19), 0xF0);
        assert_eq!(at(&one, 31, 19), 0x20, "6 px beside it is not");
        assert_eq!(at(&one, 43, 19), 0x20);
        assert_eq!(at(&one, 5, 5), 0xF0, "bit 0 of 1");
        assert_eq!(at(&one, 20, 5), 0x20, "bit 1 of 1");
        assert_eq!(at(&one, 5, 16), 0x20, "under the bit blocks");
        // A pixel is blue, green at half the shade, red, alpha.
        let dark = (19 * 300 + 299) * 4;
        assert_eq!(one.bgra[dark..dark + 4], [0x20_u8, 0x10, 0x20, 0xFF]);
        assert_eq!(one.bgra[..4], [0xF0_u8, 0x78, 0xF0, 0xFF]);
        let two = picture(2, 300, 20);
        assert_eq!(at(&two, 5, 5), 0x20, "bit 0 of 2");
        assert_eq!(at(&two, 20, 5), 0xF0, "bit 1 of 2");
        assert_eq!(at(&two, 74, 19), 0xF0, "the bar at 74");
        assert_ne!(one, two);
        assert_eq!(picture(3, 300, 20), picture(3, 300, 20));
        // Eight bits, 16 px each: x 128 shows none.
        let all = picture(0x1FF, 300, 20);
        assert_eq!(at(&all, 127, 5), 0xF0);
        assert_eq!(at(&all, 128, 5), 0x20);
        assert_eq!(picture(0, 0, 0).bgra.len(), 0);
    }

    #[test]
    fn a_sim_opens_takes_records_and_closes_its_windows() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(RECORD_FILE);
        let (mut sim, handle) = Sim::new(Some(file.clone()));
        assert_eq!(sim.editors().unwrap(), Vec::new());
        sim.live_opened();
        let window = handle.windows()[0];
        assert_eq!(sim.editors().unwrap(), vec![window]);
        let taken = sim.take(window).unwrap();
        assert_eq!(
            taken,
            Taken {
                window,
                picture: window,
                pid: 0,
                was_topmost: false,
                width: 1349,
                height: 809,
            }
        );
        assert_eq!(handle.topmost(window), Some(true));
        assert!(sim.alive(&taken));
        let first = sim.grab(&taken).unwrap();
        assert_eq!((first.width, first.height), (1349, 809));
        sim.touch(&taken, Phase::Down, (10, 20)).unwrap();
        handle.refuse(true);
        assert_eq!(
            sim.touch(&taken, Phase::Update, (11, 20)),
            Err(REFUSED.to_string())
        );
        assert_eq!(
            sim.touch(&taken, Phase::Down, (11, 20)),
            Err(REFUSED.to_string())
        );
        // An end always goes.
        sim.touch(&taken, Phase::Cancel, (10, 20)).unwrap();
        sim.touch(&taken, Phase::Up, (10, 20)).unwrap();
        sim.release(&taken);
        assert!(sim.alive(&taken), "handed back, still open in Live");
        assert_eq!(handle.topmost(window), Some(false), "z-order put back");
        sim.live_closed(&taken);
        assert!(!sim.alive(&taken), "Live closed it");
        assert!(sim.grab(&taken).is_err());
        assert!(sim.take(window).is_err());
        let expected = vec![
            json!({"op": "take", "window": 1}),
            json!({"op": "touch", "window": 1, "phase": "down", "x": 10, "y": 20}),
            json!({"op": "touch", "window": 1, "phase": "cancel", "x": 10, "y": 20}),
            json!({"op": "touch", "window": 1, "phase": "up", "x": 10, "y": 20}),
            json!({"op": "release", "window": 1}),
        ];
        assert_eq!(untimed(&handle.records()), expected);
        let lines: Vec<Value> = std::fs::read_to_string(&file)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines, handle.records());
        // Every touch carries its time, in order.
        let times: Vec<f64> = lines
            .iter()
            .filter(|r| r["op"] == "touch")
            .map(|r| r["t"].as_f64().expect("a time"))
            .collect();
        assert_eq!(times.len(), 3);
        assert!(times.is_sorted(), "{times:?}");
        assert!(times[0] >= 0.0);
    }

    /// Records without their touches' times.
    fn untimed(records: &[Value]) -> Vec<Value> {
        records
            .iter()
            .map(|r| {
                let mut r = r.clone();
                if let Some(fields) = r.as_object_mut() {
                    fields.remove("t");
                }
                r
            })
            .collect()
    }

    #[test]
    fn a_touch_is_recorded_at_its_time() {
        let (mut sim, handle) = Sim::new(None);
        sim.live_opened();
        let taken = sim.take(handle.windows()[0]).unwrap();
        sim.touch(&taken, Phase::Down, (1, 2)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        sim.touch(&taken, Phase::Up, (1, 2)).unwrap();
        let records = handle.records();
        let gap = records[2]["t"].as_f64().unwrap() - records[1]["t"].as_f64().unwrap();
        assert!((30.0..1000.0).contains(&gap), "{gap}");
        assert!(records[0].get("t").is_none(), "a take has no time");
    }

    #[test]
    fn a_host_window_stays_after_its_release_and_closes_when_asked() {
        let (mut sim, handle) = Sim::new(None);
        handle.auto_open(false);
        sim.live_opened();
        assert!(handle.windows().is_empty(), "no window opened");
        let window = handle.add_window(42);
        handle.add_window(7);
        assert_eq!(sim.windows_of(42).unwrap(), vec![window]);
        assert_eq!(sim.windows_of(5).unwrap(), Vec::new());
        let taken = sim.take(window).unwrap();
        assert_eq!(taken.pid, 42);
        sim.release(&taken);
        assert_eq!(handle.topmost(window), Some(false), "z-order put back");
        sim.live_closed(&taken);
        assert!(sim.alive(&taken), "a host's window stays");
        // Topmost before: topmost after.
        let mut again = sim.take(window).unwrap();
        again.was_topmost = true;
        sim.release(&again);
        assert_eq!(handle.topmost(window), Some(true));
        sim.close_window(&taken);
        assert!(!sim.alive(&taken));
        assert_eq!(
            handle.records().last(),
            Some(&json!({"op": "close", "window": 1}))
        );
        handle.remove_window(WindowId(2));
        assert!(handle.windows().is_empty());
        assert_eq!(handle.topmost(window), None);
    }

    #[test]
    fn a_grab_draws_its_picture_once_per_count_and_size() {
        let (mut sim, handle) = Sim::new(None);
        handle.still(true);
        sim.live_opened();
        let taken = sim.take(handle.windows()[0]).unwrap();
        let first = sim.grab(&taken).unwrap();
        assert_eq!(first, picture(0, 1349, 809));
        assert_eq!(sim.grab(&taken).unwrap(), first);
        // Another size: drawn at that size, then the first one again.
        let small = Taken {
            window: taken.window,
            picture: taken.picture,
            pid: taken.pid,
            was_topmost: false,
            width: 16,
            height: 8,
        };
        assert_eq!(sim.grab(&small).unwrap(), picture(0, 16, 8));
        assert_eq!(sim.grab(&taken).unwrap(), first);
        // Another count: drawn anew.
        handle.still(false);
        std::thread::sleep(std::time::Duration::from_millis(260));
        let later = sim.grab(&taken).unwrap();
        assert_eq!((later.width, later.height), (1349, 809));
        assert_ne!(later, first);
    }

    #[test]
    fn a_grab_takes_the_time_a_test_gives_it() {
        let (mut sim, handle) = Sim::new(None);
        handle.still(true);
        sim.live_opened();
        let taken = sim.take(handle.windows()[0]).unwrap();
        sim.grab(&taken).unwrap();
        handle.grab_delay(Duration::from_millis(30));
        let started = Instant::now();
        assert_eq!(sim.grab(&taken).unwrap(), picture(0, 1349, 809));
        assert!(started.elapsed() >= Duration::from_millis(30));
    }

    #[test]
    fn a_still_sim_shows_one_picture() {
        let (mut sim, handle) = Sim::new(None);
        handle.still(true);
        sim.live_opened();
        let taken = sim.take(handle.windows()[0]).unwrap();
        assert_eq!(sim.grab(&taken).unwrap(), picture(0, 1349, 809));
    }
}

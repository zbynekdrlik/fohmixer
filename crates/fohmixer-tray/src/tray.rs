//! The Windows glue (Tauri 2, as iemmixer's `iem-tray`): the logger, the
//! single-instance hand-over (`--exit`), the icon and its menu, and the
//! thread that polls the hub. It carries out the decisions of [`crate::args`],
//! [`crate::links`], [`crate::poll`] and [`crate::view`] and decides nothing
//! itself. Nothing here ends a process: Exit and `--exit` end this tray only.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Instant;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, RunEvent, Wry};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_RESTORE,
    SetForegroundWindow, ShowWindow,
};
use windows::core::{BOOL, PWSTR};

use crate::args::{self, Command};
use crate::links::Links;
use crate::poll::{self, HubState};
use crate::view::{self, MenuAction, OpenStep};

/// This tray's version (the workspace's).
const TRAY_VERSION: &str = fohmixer_proto::VERSION;

/// Runs the tray until the menu's Exit or a second start with `--exit`; with
/// `--exit` itself, hands the request to the running tray (or, with none
/// running, exits at once).
pub fn run() {
    let command = match args::parse(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(e) => {
            eprintln!("fohmixer-tray: {e} (usage: fohmixer-tray [--data <folder>] | --exit)");
            std::process::exit(2);
        }
    };
    let log_dir = crate::logs::log_dir(dirs::data_local_dir());
    let mut log_flush = Some(init_logging(&log_dir));
    std::panic::set_hook(Box::new(|info| {
        tracing::error!("PANIC: {info}");
    }));

    let data_dir = match &command {
        Command::Run { data_dir } => args::data_dir(
            data_dir.clone(),
            std::env::var_os("FOHMIXER_DATA"),
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        ),
        Command::Exit => PathBuf::new(),
    };
    let links = match &command {
        Command::Run { .. } => load_links(&data_dir),
        Command::Exit => Links::defaults(),
    };
    tracing::info!(
        version = %fohmixer_proto::full_version(),
        git_hash = fohmixer_proto::git_hash(),
        command = ?command,
        log_dir = %log_dir.display(),
        "fohmixer tray starting"
    );

    let exit_only = command == Command::Exit;
    let app = tauri::Builder::default()
        // First, so that a second start hands over its arguments and exits
        // before anything else is set up.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if args::is_exit_request(&argv) {
                tracing::info!(?argv, "a second start asks this tray to exit (--exit): exiting");
                app.exit(0);
            } else {
                tracing::info!(?argv, "a second start ignored: this tray runs already");
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(move |app| {
            if exit_only {
                tracing::info!("--exit: no tray was running; nothing to end");
                app.handle().exit(0);
                return Ok(());
            }
            let handle = app.handle().clone();
            let icon = app.default_window_icon().cloned();
            match build_tray(&handle, &links, icon) {
                Ok((tray, version_item)) => spawn_poll(links.version.clone(), Some(tray), Some(version_item)),
                Err(e) => {
                    tracing::error!(error = %e, "the tray icon could not be set up; polling the hub for the log only");
                    spawn_poll(links.version.clone(), None, None);
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building the fohmixer tray");

    app.run(move |_app, event| {
        if let RunEvent::Exit = event {
            tracing::info!("the tray exits");
            drop(log_flush.take());
        }
    });
}

/// The file logger: `<log_dir>\fohmixer-tray.log.<date>`, daily, no colour
/// codes; `RUST_LOG` adds filters. The guard flushes it when dropped.
fn init_logging(log_dir: &Path) -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    if let Err(e) = std::fs::create_dir_all(log_dir) {
        eprintln!("fohmixer-tray: cannot create {}: {e}", log_dir.display());
    }
    let appender = tracing_appender::rolling::daily(log_dir, crate::logs::LOG_FILE);
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive("fohmixer_tray=debug".parse().expect("a valid directive"));
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_ansi(false),
        )
        .init();
    guard
}

/// The links of the hub in `data_dir`, or the defaults (logged) when its
/// config cannot be read.
fn load_links(data_dir: &Path) -> Links {
    let links = match Links::load(data_dir) {
        Ok(links) => links,
        Err(e) => {
            tracing::error!(
                error = %e,
                "the hub's config did not load: Open fohmixer uses port 8480, Copy URL is disabled"
            );
            Links::defaults()
        }
    };
    tracing::info!(
        data_dir = %data_dir.display(),
        open = %links.open,
        version_url = %links.version,
        public = ?links.public,
        app = ?links.app,
        mbc_only = links.mbc_only,
        "the hub's links"
    );
    links
}

/// The icon with its menu: the hub's version (disabled), Open fohmixer, the
/// tag manual (#68), Copy URL, Exit; a left click opens fohmixer, the right
/// one shows the menu. Returns the icon and the version line (the poll
/// updates both).
fn build_tray(
    app: &AppHandle,
    links: &Links,
    icon: Option<tauri::image::Image<'_>>,
) -> tauri::Result<(TrayIcon, MenuItem<Wry>)> {
    let version_item = MenuItem::with_id(
        app,
        view::VERSION,
        view::version_line(&HubState::Unknown),
        false,
        None::<&str>,
    )?;
    let label = view::open_label(links.mbc_only);
    let open_item = MenuItem::with_id(app, view::OPEN, label, true, None::<&str>)?;
    let manual_item = MenuItem::with_id(app, view::MANUAL, "Tag manual", true, None::<&str>)?;
    let (copy_label, copy_enabled) = view::copy_item(links.public.as_deref());
    let copy_item = MenuItem::with_id(app, view::COPY, copy_label, copy_enabled, None::<&str>)?;
    let exit_item = MenuItem::with_id(
        app,
        view::EXIT,
        "Quit tray (hub keeps running)",
        true,
        None::<&str>,
    )?;
    let separator1 = PredefinedMenuItem::separator(app)?;
    let separator2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &version_item,
            &separator1,
            &open_item,
            &manual_item,
            &copy_item,
            &separator2,
            &exit_item,
        ],
    )?;

    let links = links.clone();
    let click_links = links.clone();
    // When the tray last started the app window (#280), shared by the click
    // and the menu item.
    let started: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let click_started = Arc::clone(&started);
    // When a left click last opened fohmixer (a double click opens one tab).
    let opened: Mutex<Option<Instant>> = Mutex::new(None);
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip(view::tooltip(&HubState::Unknown, TRAY_VERSION))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(move |tray, event| {
            let TrayIconEvent::Click {
                button,
                button_state,
                ..
            } = event
            else {
                return;
            };
            let mut last = opened.lock().unwrap_or_else(PoisonError::into_inner);
            let since = (*last).map(|t| u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX));
            if view::click_opens(
                matches!(button, MouseButton::Left),
                matches!(button_state, MouseButtonState::Up),
                since,
            ) {
                *last = Some(Instant::now());
                open_app(tray.app_handle(), &click_links, &click_started, label);
            }
        })
        .on_menu_event(move |app, event| match view::action(event.id.as_ref()) {
            Some(MenuAction::Open) => open_app(app, &links, &started, label),
            Some(MenuAction::Manual) => open(app, &links.manual, "Tag manual"),
            Some(MenuAction::Copy) => match &links.public {
                Some(url) => copy(app, url),
                None => {
                    tracing::warn!("Copy URL chosen without a public URL (the item is disabled)")
                }
            },
            Some(MenuAction::Exit) => {
                tracing::info!(
                    "Exit chosen in the tray menu: the tray exits, the hub keeps running"
                );
                app.exit(0);
            }
            None => tracing::debug!(id = ?event.id, "a menu event without an action"),
        });
    match icon {
        Some(icon) => builder = builder.icon(icon),
        None => tracing::warn!("no icon in the tray's context: the tray icon has no picture"),
    }
    let tray = builder.build(app)?;
    tracing::info!(copy_url = copy_enabled, "the tray icon is set up");
    Ok((tray, version_item))
}

/// The Open item and the left click (#280): with `[tray] app`, the app
/// window brought to the front when it is open, else the command started
/// (once per [`view::START_GAP_MS`] while its window does not show); without
/// it, the hub's local URL in the default browser. `label` names the item.
fn open_app(app: &AppHandle, links: &Links, started: &Mutex<Option<Instant>>, label: &str) {
    let Some(command) = &links.app else {
        open(app, &links.open, label);
        return;
    };
    let window = app_window(&command[0]);
    let mut last = started.lock().unwrap_or_else(PoisonError::into_inner);
    let since = (*last).map(|t| u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX));
    match (view::open_step(window.is_some(), since), window) {
        (OpenStep::Focus, Some(window)) => focus(window, label),
        (OpenStep::Wait, _) => tracing::info!(
            since_start_ms = since,
            "{label}: the app window started moments ago does not show yet; nothing started again"
        ),
        _ => {
            *last = Some(Instant::now());
            start(command, label);
        }
    }
}

/// The first visible top-level window that is the app window
/// ([`view::is_app_window`]: the page's title, in a process of `program`).
fn app_window(program: &str) -> Option<HWND> {
    let mut list: Vec<HWND> = Vec::new();
    // SAFETY: `collect` writes only into `list`, which outlives the call.
    if let Err(e) =
        unsafe { EnumWindows(Some(collect), LPARAM(&mut list as *mut Vec<HWND> as isize)) }
    {
        tracing::warn!(error = %e, "EnumWindows failed: no app window found");
        return None;
    }
    list.into_iter().find(|&window| {
        // SAFETY: plain queries of a handle EnumWindows gave.
        unsafe { IsWindowVisible(window) }.as_bool()
            && view::is_app_window(&title_of(window), &image_of(window), program)
    })
}

unsafe extern "system" fn collect(window: HWND, data: LPARAM) -> BOOL {
    // SAFETY: `data` is the `Vec<HWND>` `app_window` lent for the call.
    let list = unsafe { &mut *(data.0 as *mut Vec<HWND>) };
    list.push(window);
    BOOL(1)
}

fn title_of(window: HWND) -> String {
    let mut text = [0_u16; 256];
    // SAFETY: the buffer is ours; the call writes at most its length.
    let length = unsafe { GetWindowTextW(window, &mut text) };
    String::from_utf16_lossy(&text[..usize::try_from(length).unwrap_or(0)])
}

/// The full path of the window's process image, `""` when it cannot be read
/// (which matches no program).
fn image_of(window: HWND) -> String {
    let mut pid = 0_u32;
    // SAFETY: `pid` outlives the call.
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    // SAFETY: a query-only handle, closed below.
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
    else {
        return String::new();
    };
    let mut path = [0_u16; 1024];
    let mut size = u32::try_from(path.len()).unwrap_or(0);
    // SAFETY: the buffer and its size are ours; the call writes at most `size`.
    let read = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut size,
        )
    };
    // SAFETY: the handle OpenProcess gave, closed once.
    let _ = unsafe { CloseHandle(process) };
    match read {
        Ok(()) => String::from_utf16_lossy(&path[..usize::try_from(size).unwrap_or(0)]),
        Err(_) => String::new(),
    }
}

/// The app window to the front, restored first when minimized (a maximized
/// window keeps its size).
fn focus(window: HWND, label: &str) {
    // SAFETY: plain calls on a window handle (a closed one only fails).
    let minimized = unsafe { IsIconic(window) }.as_bool();
    if minimized {
        let _ = unsafe { ShowWindow(window, SW_RESTORE) };
    }
    let front = unsafe { SetForegroundWindow(window) }.as_bool();
    if front {
        tracing::info!(
            minimized,
            "{label}: the open app window brought to the front"
        );
    } else {
        tracing::warn!(
            minimized,
            "{label}: the app window is open, Windows did not bring it to the front"
        );
    }
}

/// Starts the app command (no shell; the program, then its arguments).
fn start(command: &[String], label: &str) {
    let spawned = std::process::Command::new(&command[0])
        .args(&command[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        Ok(child) => tracing::info!(
            ?command,
            pid = child.id(),
            "{label}: no app window open, the app started"
        ),
        Err(e) => tracing::error!(?command, error = %e, "{label}: the app did not start"),
    }
}

/// Opens one of the hub's local pages in the default browser (`what` names
/// the menu item in the log: Open fohmixer, the tag manual).
fn open(app: &AppHandle, url: &str, what: &str) {
    match app.opener().open_url(url, None::<&str>) {
        Ok(()) => tracing::info!(url, "{what}: opened in the default browser"),
        Err(e) => tracing::error!(url, error = %e, "{what}: the browser did not open"),
    }
}

/// Copy URL: the public URL to the clipboard.
fn copy(app: &AppHandle, url: &str) {
    match app.clipboard().write_text(url.to_string()) {
        Ok(()) => tracing::info!(url, "Copy URL: copied to the clipboard"),
        Err(e) => tracing::error!(url, error = %e, "Copy URL: the clipboard write failed"),
    }
}

/// The thread that polls the hub every [`poll::EVERY`] and shows a changed
/// state (one log line per change, not per poll).
fn spawn_poll(url: String, tray: Option<TrayIcon>, version_item: Option<MenuItem<Wry>>) {
    let spawned = thread::Builder::new()
        .name("hub-poll".into())
        .spawn(move || {
            let mut last = HubState::Unknown;
            loop {
                let next = poll::poll(&url, poll::TIMEOUT);
                if view::changed(&last, &next) {
                    log_state(&next);
                    show(tray.as_ref(), version_item.as_ref(), &next);
                }
                last = next;
                thread::sleep(poll::EVERY);
            }
        });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "the hub poll did not start: the tray shows no state");
    }
}

fn log_state(state: &HubState) {
    match state {
        HubState::Unknown => {}
        HubState::Up(info) => tracing::info!(
            version = %info.version,
            git_hash = %info.git_hash,
            branch = %info.branch,
            "the hub answers"
        ),
        HubState::Down { reason, detail } => {
            tracing::warn!(reason = %reason, detail = %detail, "the hub does not answer")
        }
    }
}

fn show(tray: Option<&TrayIcon>, version_item: Option<&MenuItem<Wry>>, state: &HubState) {
    if let Some(tray) = tray
        && let Err(e) = tray.set_tooltip(Some(view::tooltip(state, TRAY_VERSION)))
    {
        tracing::warn!(error = %e, "the tooltip did not update");
    }
    if let Some(item) = version_item
        && let Err(e) = item.set_text(view::version_line(state))
    {
        tracing::warn!(error = %e, "the version line did not update");
    }
}

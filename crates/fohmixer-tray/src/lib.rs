//! The fohmixer tray on the Ableton PC (#39), shaped like iemmixer's
//! `iem-tray`: an icon with the hub's state in its tooltip, and a menu with
//! the hub's version, **Open fohmixer** (the hub's local URL in the default
//! browser), **Copy URL** (the public URL, when remote access is set up) and
//! **Exit** (the tray only, never the hub).
//!
//! With `[tray] app` in the hub's config (#280, the MBC PC: a Brave app
//! window on the local hub) Open never opens a browser tab: it brings the
//! open app window (the page's title, in the app's program) to the front, or
//! starts the command; on a hub that serves the MBC Live alone the item says
//! **Open MBC mixer**.
//!
//! The tray runs no server and changes nothing in the hub: it reads the
//! hub's config (`<data>\fohmixer-hub.toml`, [`links`]) and polls its public
//! `GET /api/version` ([`poll`]); the instances and the layout need a login,
//! so they stay on the surface. Every decision is a pure function here,
//! tested on every platform: the arguments ([`args`]), the links, the poll's
//! reading of an answer, and what the tooltip and the menu say ([`view`]).
//! The Tauri glue (`tray.rs`, Windows only) carries them out and decides
//! nothing.
//!
//! The installer runs it at the band user's logon (`\fohmixer\fohmixer-tray`)
//! and ends it before an update with a second start `fohmixer-tray.exe
//! --exit` (`\fohmixer\fohmixer-tray-stop`): the running tray gets that
//! argument through the single-instance plugin and exits (spec I7: nothing is
//! ended by force). The log is
//! `%LOCALAPPDATA%\fohmixer\logs\fohmixer-tray.log.<date>` ([`logs`]).

pub mod args;
pub mod links;
pub mod logs;
pub mod poll;
pub mod promote;
pub mod view;

#[cfg(windows)]
mod tray;

#[cfg(windows)]
pub use tray::run;

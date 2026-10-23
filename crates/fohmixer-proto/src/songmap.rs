//! The song map (#140, spec §4 and Appendix A): a Live set's locators and
//! its `Sections` track's clips turned into songs, their spans and keys.
//! This first part is the key token and its values ([`key`]).

pub mod key;

pub use key::{Key, KeyError, parse_token};

//! Where the tray logs: `%LOCALAPPDATA%\fohmixer\logs\fohmixer-tray.log.<date>`
//! (a daily file, as iemmixer's `iem-tray.log.<date>`), in the band user's
//! own profile: the tray runs in that user's session.

use std::path::PathBuf;

/// The daily log file's name before its `.<date>`.
pub const LOG_FILE: &str = "fohmixer-tray.log";

/// The log folder under the user's local application data folder (`local`,
/// `dirs::data_local_dir()`); the working folder's `logs` without one.
pub fn log_dir(local: Option<PathBuf>) -> PathBuf {
    match local {
        Some(local) => local.join("fohmixer").join("logs"),
        None => PathBuf::from("logs"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_go_to_fohmixer_logs_under_local_app_data() {
        assert_eq!(
            log_dir(Some(PathBuf::from("local"))),
            PathBuf::from("local").join("fohmixer").join("logs")
        );
    }

    #[test]
    fn without_local_app_data_the_logs_stay_in_the_working_folder() {
        assert_eq!(log_dir(None), PathBuf::from("logs"));
    }

    #[test]
    fn the_file_is_named_after_the_tray() {
        assert_eq!(LOG_FILE, "fohmixer-tray.log");
    }
}

//! The tray's command line: `fohmixer-tray.exe [--data <folder>]` runs the
//! tray; `fohmixer-tray.exe --exit` asks a running tray to exit (the
//! installer's `\fohmixer\fohmixer-tray-stop` task runs it in the band user's
//! session; the single-instance plugin hands the arguments to the running
//! tray, [`is_exit_request`]).

use std::ffi::OsString;
use std::path::PathBuf;

/// Asks the running tray to exit.
pub const EXIT_FLAG: &str = "--exit";
/// The hub's data folder (where `fohmixer-hub.toml` is).
pub const DATA_FLAG: &str = "--data";

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Run the tray; the data folder when `--data` names one.
    Run { data_dir: Option<PathBuf> },
    /// End the running tray (and nothing else).
    Exit,
}

/// Reads the arguments after the program name. `--exit` stands alone;
/// `--data` takes one folder, once; anything else is refused with a message
/// naming it.
pub fn parse<I>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = String>,
{
    let mut exit = false;
    let mut data_dir: Option<PathBuf> = None;
    let mut rest = args.into_iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            EXIT_FLAG if !exit => exit = true,
            DATA_FLAG if data_dir.is_none() => match rest.next() {
                Some(dir) if !dir.is_empty() => data_dir = Some(PathBuf::from(dir)),
                _ => return Err(format!("{DATA_FLAG} needs a folder")),
            },
            other => return Err(format!("argument refused: {other:?}")),
        }
    }
    match (exit, data_dir) {
        (true, None) => Ok(Command::Exit),
        (true, Some(_)) => Err(format!("{EXIT_FLAG} takes no other argument")),
        (false, data_dir) => Ok(Command::Run { data_dir }),
    }
}

/// Whether a second start's whole command line (the program name first, as
/// the single-instance plugin passes it) asks the running tray to exit.
pub fn is_exit_request(argv: &[String]) -> bool {
    parse(argv.iter().skip(1).cloned()) == Ok(Command::Exit)
}

/// The hub's data folder, found as the hub finds it: `--data`, else
/// `FOHMIXER_DATA` (a non-empty value), else the working folder.
pub fn data_dir(given: Option<PathBuf>, env: Option<OsString>, cwd: PathBuf) -> PathBuf {
    given
        .or_else(|| env.filter(|v| !v.is_empty()).map(PathBuf::from))
        .unwrap_or(cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_argument_runs_the_tray_without_a_data_folder() {
        assert_eq!(parse(args(&[])), Ok(Command::Run { data_dir: None }));
    }

    #[test]
    fn data_names_the_folder() {
        assert_eq!(
            parse(args(&["--data", r"C:\ProgramData\fohmixer"])),
            Ok(Command::Run {
                data_dir: Some(PathBuf::from(r"C:\ProgramData\fohmixer"))
            })
        );
    }

    #[test]
    fn exit_alone_asks_the_running_tray_to_exit() {
        assert_eq!(parse(args(&["--exit"])), Ok(Command::Exit));
    }

    #[test]
    fn exit_with_another_argument_is_refused() {
        assert_eq!(
            parse(args(&["--exit", "--data", "x"])),
            Err("--exit takes no other argument".to_string())
        );
        assert_eq!(
            parse(args(&["--data", "x", "--exit"])),
            Err("--exit takes no other argument".to_string())
        );
    }

    #[test]
    fn a_repeated_flag_is_refused() {
        assert_eq!(
            parse(args(&["--exit", "--exit"])),
            Err("argument refused: \"--exit\"".to_string())
        );
        assert_eq!(
            parse(args(&["--data", "a", "--data", "b"])),
            Err("argument refused: \"--data\"".to_string())
        );
    }

    #[test]
    fn data_without_a_folder_is_refused() {
        assert_eq!(
            parse(args(&["--data"])),
            Err("--data needs a folder".to_string())
        );
        assert_eq!(
            parse(args(&["--data", ""])),
            Err("--data needs a folder".to_string())
        );
    }

    #[test]
    fn an_unknown_argument_is_refused_by_name() {
        assert_eq!(
            parse(args(&["--quit"])),
            Err("argument refused: \"--quit\"".to_string())
        );
        assert_eq!(
            parse(args(&["x"])),
            Err("argument refused: \"x\"".to_string())
        );
    }

    #[test]
    fn a_second_start_with_exit_is_an_exit_request() {
        assert!(is_exit_request(&args(&[
            r"C:\app\fohmixer-tray.exe",
            "--exit"
        ])));
    }

    #[test]
    fn a_second_start_without_exit_is_not_an_exit_request() {
        assert!(!is_exit_request(&args(&[r"C:\app\fohmixer-tray.exe"])));
        assert!(!is_exit_request(&args(&[
            r"C:\app\fohmixer-tray.exe",
            "--data",
            "x"
        ])));
        assert!(!is_exit_request(&args(&[
            r"C:\app\fohmixer-tray.exe",
            "--exit",
            "x"
        ])));
        // The program name itself is never read as a flag.
        assert!(!is_exit_request(&args(&["--exit"])));
        assert!(!is_exit_request(&[]));
    }

    #[test]
    fn the_data_folder_is_the_flag_then_the_variable_then_the_working_folder() {
        let cwd = PathBuf::from("cwd");
        assert_eq!(
            data_dir(
                Some(PathBuf::from("flag")),
                Some(OsString::from("env")),
                cwd.clone()
            ),
            PathBuf::from("flag")
        );
        assert_eq!(
            data_dir(None, Some(OsString::from("env")), cwd.clone()),
            PathBuf::from("env")
        );
        assert_eq!(data_dir(None, Some(OsString::new()), cwd.clone()), cwd);
        assert_eq!(data_dir(None, None, cwd.clone()), cwd);
    }
}

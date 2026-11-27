//! The tray's app window command (#280, `fohmixer-hub.toml` `[tray] app`):
//! the one check the hub's config and the tray's reading of it share.

/// Why an app command is refused (a program first, then its arguments):
/// `None` when it is usable.
pub fn app_problem(app: &[String]) -> Option<&'static str> {
    if app.is_empty() {
        Some("[tray] app names no program")
    } else if app.iter().any(String::is_empty) {
        Some("[tray] app holds an empty word")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_program_with_its_arguments_is_usable() {
        assert_eq!(app_problem(&words(&["brave.exe"])), None);
        assert_eq!(app_problem(&words(&["brave.exe", "--app=x"])), None);
    }

    #[test]
    fn no_program_or_an_empty_word_is_refused() {
        assert_eq!(app_problem(&[]), Some("[tray] app names no program"));
        assert_eq!(
            app_problem(&words(&[""])),
            Some("[tray] app holds an empty word")
        );
        assert_eq!(
            app_problem(&words(&["brave.exe", ""])),
            Some("[tray] app holds an empty word")
        );
    }
}

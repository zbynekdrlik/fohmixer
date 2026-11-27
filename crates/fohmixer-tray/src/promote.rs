//! Keeps the tray's icon visible after an update (#280). Windows 11 keeps
//! one notification-icon entry per executable path
//! (`HKCU\Control Panel\NotifyIconSettings\<id>`: `ExecutablePath`,
//! `IsPromoted`), and each fohmixer version runs its tray from its own
//! `<DataDir>\app\<version>\fohmixer-tray.exe`, so each update makes a new
//! entry that Windows shows hidden. [`decide`] carries the choice over to the
//! new entry; the glue (`tray.rs`) reads the entries, writes the value and
//! adds the icon again so Explorer reads it.

/// One notification-icon entry as Windows keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The entry's key name under `NotifyIconSettings`.
    pub key: String,
    /// `ExecutablePath`.
    pub exe: String,
    /// `IsPromoted`; none while nobody set it.
    pub promoted: Option<u32>,
}

/// What to do with this tray's own entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Promote {
    /// The own entry is not there yet (Explorer writes it after the icon is
    /// added): read again later.
    Wait,
    /// Leave it: it has a value already, or the user hid the earlier icon.
    Leave,
    /// Write `IsPromoted` = 1 on this key.
    Set(String),
}

/// The decision for the tray running from `own_exe`: wait while its entry is
/// missing; leave an entry that has a value; else follow the newest earlier
/// fohmixer tray entry that has one (by the version in its folder name): 0
/// leaves it hidden, 1 or no earlier choice at all promotes it.
pub fn decide(entries: &[Entry], own_exe: &str) -> Promote {
    let _ = (entries, own_exe);
    Promote::Wait
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: &str = r"C:\ProgramData\fohmixer\app\1.60.9\fohmixer-tray.exe";

    fn entry(key: &str, version: &str, promoted: Option<u32>) -> Entry {
        Entry {
            key: key.to_string(),
            exe: format!(r"C:\ProgramData\fohmixer\app\{version}\fohmixer-tray.exe"),
            promoted,
        }
    }

    #[test]
    fn waits_while_its_own_entry_is_missing() {
        let entries = [entry("1", "1.60.8", Some(1))];
        assert_eq!(decide(&entries, OWN), Promote::Wait);
        assert_eq!(decide(&[], OWN), Promote::Wait);
    }

    #[test]
    fn promotes_a_new_entry_when_the_newest_earlier_choice_was_shown() {
        let entries = [
            entry("old", "1.47.0", Some(1)),
            entry("mid", "1.54.3", None),
            entry("prev", "1.60.8", Some(1)),
            entry("own", "1.60.9", None),
        ];
        assert_eq!(decide(&entries, OWN), Promote::Set("own".into()));
    }

    #[test]
    fn promotes_a_new_entry_when_no_earlier_entry_has_a_choice() {
        let entries = [entry("a", "1.54.2", None), entry("own", "1.60.9", None)];
        assert_eq!(decide(&entries, OWN), Promote::Set("own".into()));
        assert_eq!(
            decide(&[entry("own", "1.60.9", None)], OWN),
            Promote::Set("own".into())
        );
    }

    #[test]
    fn keeps_it_hidden_when_the_user_hid_the_newest_earlier_icon() {
        // 1.60.8 is newer than 1.9.0 by version, though not as text.
        let entries = [
            entry("shown", "1.9.0", Some(1)),
            entry("hidden", "1.60.8", Some(0)),
            entry("own", "1.60.9", None),
        ];
        assert_eq!(decide(&entries, OWN), Promote::Leave);
        let entries = [
            entry("hidden", "1.9.0", Some(0)),
            entry("shown", "1.60.8", Some(1)),
            entry("own", "1.60.9", None),
        ];
        assert_eq!(decide(&entries, OWN), Promote::Set("own".into()));
    }

    #[test]
    fn leaves_an_own_entry_that_has_a_value() {
        let entries = [
            entry("prev", "1.60.8", Some(1)),
            entry("own", "1.60.9", Some(0)),
        ];
        assert_eq!(decide(&entries, OWN), Promote::Leave);
        let entries = [entry("own", "1.60.9", Some(1))];
        assert_eq!(decide(&entries, OWN), Promote::Leave);
    }

    #[test]
    fn matches_its_own_path_without_case_and_ignores_other_programs() {
        let mut own = entry("own", "1.60.9", None);
        own.exe = own.exe.to_uppercase();
        let other = Entry {
            key: "brave".into(),
            exe: r"C:\Program Files\Brave\brave.exe".into(),
            promoted: Some(0),
        };
        let entries = [other, own];
        assert_eq!(decide(&entries, OWN), Promote::Set("own".into()));
    }

    #[test]
    fn a_newer_entry_than_its_own_is_not_an_earlier_choice() {
        // A downgrade: the newer version's hidden icon is not the user's
        // latest choice before this one, the older shown one is.
        let entries = [
            entry("older", "1.60.8", Some(1)),
            entry("newer", "1.61.0", Some(0)),
            entry("own", "1.60.9", None),
        ];
        assert_eq!(decide(&entries, OWN), Promote::Set("own".into()));
    }
}

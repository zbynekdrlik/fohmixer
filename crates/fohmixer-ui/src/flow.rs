//! What a control is to the surface: a strip-wide column or a button, and
//! the Live instance a group's strips share (#63: the group's title names
//! it once). Where the controls go is `arrange` (the control column, #63).

use fohmixer_proto::layout::Control;

/// Whether a control is drawn as a strip-wide column (a strip, a parameter
/// fader); the other controls are buttons.
pub fn is_column(control: &Control) -> bool {
    matches!(control, Control::Strip(_) | Control::ParamFader { .. })
}

/// The Live instance every strip of a group shares (#63: the group's title
/// names it once instead of each strip); none when the group holds no strip
/// or its strips differ. The other controls do not count.
pub fn shared_instance(controls: &[Control]) -> Option<String> {
    let mut instances = controls.iter().filter_map(|c| match c {
        Control::Strip(strip) => Some(strip.binding.instance.as_str()),
        _ => None,
    });
    let first = instances.next()?;
    instances.all(|i| i == first).then(|| first.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::layout::{Binding, Press, Strip, StripKind};

    fn strip(wide: bool) -> Control {
        Control::Strip(Box::new(Strip {
            binding: Binding {
                instance: "band".into(),
                anchor: fohmixer_proto::layout::Anchor::Master,
                path: None,
            },
            strip_kind: StripKind::Standard,
            wide,
            mute_guard: false,
            pinned: false,
            label: None,
            mark: None,
        }))
    }

    fn toggle() -> Control {
        Control::ParamToggle {
            label: "Vox 1 TU".into(),
            targets: vec![],
            press: Press::Toggle,
            color: None,
        }
    }

    fn fader() -> Control {
        Control::ParamFader {
            label: "All".into(),
            targets: vec![],
        }
    }

    fn strip_of(instance: &str) -> Control {
        let Control::Strip(mut s) = strip(false) else {
            unreachable!()
        };
        s.binding.instance = instance.into();
        Control::Strip(s)
    }

    #[test]
    fn a_group_names_its_instance_only_when_every_strip_shares_it() {
        assert_eq!(shared_instance(&[]), None, "no strip");
        assert_eq!(shared_instance(&[toggle(), fader()]), None, "no strip");
        assert_eq!(shared_instance(&[strip_of("band")]), Some("band".into()));
        assert_eq!(
            shared_instance(&[strip_of("master"), toggle(), strip_of("master")]),
            Some("master".into()),
            "the other controls do not count"
        );
        assert_eq!(
            shared_instance(&[strip_of("band"), strip_of("master")]),
            None
        );
        assert_eq!(
            shared_instance(&[strip_of("band"), strip_of("band"), strip_of("master")]),
            None,
            "a later strip differs"
        );
    }

    #[test]
    fn strips_and_parameter_faders_are_columns() {
        assert!(is_column(&strip(false)));
        assert!(is_column(&fader()));
        assert!(!is_column(&toggle()));
    }
}

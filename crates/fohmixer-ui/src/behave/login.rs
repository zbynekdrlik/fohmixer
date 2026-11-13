//! The login line at the column's foot (#184): which login a tablet holds,
//! where that is worth a line. The MBC login always says so; the engineer
//! only on a hub that serves one Live set (the MBC hub's page), never on the
//! FOH page, which stays as it was.

use fohmixer_proto::layout::Layout;
use fohmixer_proto::roles::Role;
use fohmixer_proto::sectors::pool;

/// The line for `role` (none before the hub's hello) on a page whose
/// faders all belong to one Live set (`one_instance`).
pub fn login_line(role: Option<Role>, one_instance: bool) -> Option<&'static str> {
    match role? {
        Role::Mbc => Some("logged in: MBC"),
        Role::Engineer => one_instance.then_some("logged in: ENGINEER"),
    }
}

/// Whether every pool fader of `layout` belongs to one Live set (a layout
/// with no fader counts as one: nothing on it is another set's).
pub fn one_instance(layout: &Layout) -> bool {
    let faders = pool(layout);
    faders
        .first()
        .is_none_or(|first| faders.iter().all(|p| p.id.instance == first.id.instance))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_names_the_role_where_it_matters() {
        assert_eq!(login_line(Some(Role::Mbc), true), Some("logged in: MBC"));
        assert_eq!(login_line(Some(Role::Mbc), false), Some("logged in: MBC"));
        assert_eq!(
            login_line(Some(Role::Engineer), true),
            Some("logged in: ENGINEER")
        );
        assert_eq!(login_line(Some(Role::Engineer), false), None);
        assert_eq!(login_line(None, true), None);
        assert_eq!(login_line(None, false), None);
    }

    fn layout(instances: &[&str]) -> Layout {
        let controls: Vec<serde_json::Value> = instances
            .iter()
            .enumerate()
            .map(|(i, instance)| {
                serde_json::json!({"kind": "strip", "strip_kind": "standard",
                    "binding": {"instance": instance,
                        "anchor": {"kind": "track", "name": format!("Hand {i}")}}})
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "schema": 2, "default_page": "p",
            "pages": [{"id": "p", "title": "P", "rail": [], "rows": [{"sections": [
                {"kind": "group", "id": "g", "title": "G", "color": "#1E3A1E",
                 "controls": controls}]}]}]
        }))
        .unwrap()
    }

    #[test]
    fn one_instance_is_every_fader_of_one_live_set() {
        assert!(one_instance(&layout(&["mbc", "mbc"])));
        assert!(one_instance(&layout(&["mbc"])));
        assert!(one_instance(&layout(&[])));
        assert!(!one_instance(&layout(&["mbc", "band"])));
        assert!(!one_instance(&layout(&["band", "master", "band"])));
    }
}

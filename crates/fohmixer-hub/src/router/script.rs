//! The router's part of `script missing` (#184 PR D, spec §6.5): the
//! script watch's verdict on a local instance (`crate::script_watch`) is
//! kept, told to every page with the instance's state
//! (`ServerMsg::Instance.script_missing`), shown in `/api/status` and
//! recorded once per change (`script`, warn class) with one log line.

use serde_json::{Value, json};

use super::Router;
use crate::script_watch::change_line;

/// The `script` record's fields.
pub(super) fn script_fields(instance: &str, missing: bool) -> Value {
    json!({"instance": instance, "missing": missing})
}

/// Whether an instance is told and shown `script_missing`: only while it
/// is offline (the watch's verdict trails a connect by up to one look).
pub fn shown(online: bool, missing: bool) -> bool {
    !online && missing
}

impl Router {
    /// The watch says `instance`'s script is (no longer) missing: kept,
    /// recorded and told only when it changes the kept verdict, and only
    /// for an instance of this hub.
    pub(super) fn script_verdict(&mut self, instance: &str, missing: bool) {
        if !self.live.contains_key(instance) {
            return;
        }
        let changed = if missing {
            self.script_missing.insert(instance.to_string())
        } else {
            self.script_missing.remove(instance)
        };
        if !changed {
            return;
        }
        self.io
            .events
            .record("script", script_fields(instance, missing));
        tracing::warn!(instance, missing, "{}", change_line(missing));
        self.broadcast_instance(instance);
    }
}

#[cfg(test)]
mod tests {
    use fohmixer_proto::client::ServerMsg;

    use super::super::RouterMsg;
    use super::super::tests::{attach, offline_router, records, status};

    fn instance(script_missing: bool) -> ServerMsg {
        ServerMsg::Instance {
            name: "band".into(),
            online: false,
            busy: false,
            set_name: String::new(),
            linked: false,
            script_missing,
        }
    }

    fn verdict(instance: &str, missing: bool) -> RouterMsg {
        RouterMsg::ScriptMissing {
            instance: instance.into(),
            missing,
        }
    }

    #[tokio::test]
    async fn a_missing_script_is_told_recorded_and_shown_once_per_change() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, events) = offline_router(dir.path());
        let page = attach(&mut router, 1);
        let _ = page.take();
        let _ = records(&events);
        assert!(router.handle(verdict("band", true)));
        assert_eq!(page.take().unwrap(), vec![instance(true)]);
        let written = records(&events);
        assert_eq!(written.len(), 1);
        assert_eq!(written[0]["ev"], "script");
        assert_eq!(written[0]["instance"], "band");
        assert_eq!(written[0]["missing"], true);
        assert!(crate::events::is_warn(&written[0]));
        assert!(status(&mut router).script_missing.contains("band"));
        // A page attached now hears it too.
        let late = attach(&mut router, 2);
        assert!(late.take().unwrap().contains(&instance(true)));
        // The same verdict again: nothing.
        assert!(router.handle(verdict("band", true)));
        assert_eq!(page.take().unwrap_or_default(), Vec::<ServerMsg>::new());
        assert!(records(&events).is_empty());
        // Back: told and recorded, gone from the status.
        assert!(router.handle(verdict("band", false)));
        assert_eq!(page.take().unwrap(), vec![instance(false)]);
        let written = records(&events);
        assert_eq!(written.len(), 1);
        assert_eq!(written[0]["missing"], false);
        assert!(status(&mut router).script_missing.is_empty());
        assert!(router.handle(verdict("band", false)));
        assert_eq!(page.take().unwrap_or_default(), Vec::<ServerMsg>::new());
        assert!(records(&events).is_empty());
    }

    #[test]
    fn script_missing_is_shown_only_while_offline() {
        assert!(super::shown(false, true));
        assert!(!super::shown(true, true));
        assert!(!super::shown(false, false));
        assert!(!super::shown(true, false));
    }

    #[tokio::test]
    async fn an_instance_the_hub_does_not_hold_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, events) = offline_router(dir.path());
        let page = attach(&mut router, 1);
        let _ = page.take();
        let _ = records(&events);
        assert!(router.handle(verdict("mbc", true)));
        assert_eq!(page.take().unwrap_or_default(), Vec::<ServerMsg>::new());
        assert!(records(&events).is_empty());
        assert!(status(&mut router).script_missing.is_empty());
    }
}

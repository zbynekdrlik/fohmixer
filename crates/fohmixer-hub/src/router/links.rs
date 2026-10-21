//! The router's half of the fader links' switches (#130, D18): each
//! layout's leaders, the hub values every page is told, a page's switch
//! (recorded, saved, told; `rules::links` decides).

use fohmixer_proto::client::{ServerMsg, fader_link_key, fader_link_of};
use fohmixer_proto::layout::LinkId;
use serde_json::{Value, json};

use super::Router;
use crate::live::subs::ClientId;
use crate::outbox::Outbox;
use crate::rules::links::link_line;

impl Router {
    /// A new layout's leaders: the new ones' switches to every page.
    pub(super) fn links_layout(&mut self, leaders: Vec<LinkId>) {
        for (link, on) in self.links.set_leaders(leaders) {
            for outbox in self.clients.values() {
                outbox.hub(&fader_link_key(&link), json!(on));
            }
        }
    }

    /// A new page is told every leader's switch.
    pub(super) fn links_attach(&self, outbox: &Outbox) {
        for (link, on) in self.links.values() {
            outbox.hub(&fader_link_key(&link), json!(on));
        }
    }

    /// A page's `set_hub` of a fader link's key: whether the key was one.
    pub(super) fn links_set_hub(&mut self, client: ClientId, key: &str, value: &Value) -> bool {
        let Some(link) = fader_link_of(key) else {
            return false;
        };
        let switched = value
            .as_bool()
            .and_then(|on| self.links.switch(&link, on).map(|changed| (on, changed)));
        let Some((on, changed)) = switched else {
            if let Some(outbox) = self.clients.get(&client) {
                outbox.reply(ServerMsg::Error {
                    id: None,
                    message: format!("no fader link {key:?} taking {value}"),
                });
            }
            return true;
        };
        self.io.events.record(
            "fader_link",
            json!({"instance": link.instance, "id": link.id, "on": on, "client": client, "peer": self.peers.get(&client)}),
        );
        if changed {
            tracing::info!(client, "{}", link_line(&link, on));
            self.state.fader_links = self.links.saved();
            self.save_state();
        }
        for outbox in self.clients.values() {
            outbox.hub(key, json!(on));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use fohmixer_proto::client::ServerMsg;
    use fohmixer_proto::layout::LinkId;
    use serde_json::{Value, json};

    use super::super::RouterMsg;
    use super::super::tests::{attach, offline_router, offline_router_with};
    use crate::outbox::Outbox;
    use crate::rules::HubState;
    use crate::rules::reverb::{Saved, SavedFader, SavedLow};

    const REV: &str = "fader_link|band|REV";

    fn rev() -> LinkId {
        LinkId {
            instance: "band".into(),
            id: "REV".into(),
        }
    }

    fn layout(rev: u64, fader_leaders: Vec<LinkId>) -> RouterMsg {
        RouterMsg::Layout {
            rev,
            stage: None,
            targets: BTreeMap::new(),
            strips: Vec::new(),
            reverb: Vec::new(),
            fader_leaders,
            autotune: Vec::new(),
        }
    }

    /// The fader links' hub values waiting in `outbox`.
    fn hubs(outbox: &Outbox) -> Vec<(String, Value)> {
        outbox
            .take()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|m| match m {
                ServerMsg::Hub { key, value } if key.starts_with("fader_link|") => {
                    Some((key, value))
                }
                _ => None,
            })
            .collect()
    }

    /// `hub-state.json` as the router last saved it.
    fn saved(dir: &std::path::Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(dir.join("hub-state.json")).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn each_leaders_switch_is_told_saved_and_kept_beside_stage_aut() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, records) = offline_router(dir.path());
        let first = attach(&mut router, 1);
        let second = attach(&mut router, 2);
        assert_eq!(hubs(&first), vec![], "no layout, no leader");
        router.handle(layout(1, vec![rev()]));
        assert_eq!(hubs(&first), vec![(REV.to_string(), json!(true))]);
        assert_eq!(hubs(&second), vec![(REV.to_string(), json!(true))]);
        router.handle(RouterMsg::SetHub {
            client: 2,
            key: REV.into(),
            value: json!(false),
        });
        assert_eq!(hubs(&first), vec![(REV.to_string(), json!(false))]);
        assert_eq!(
            saved(dir.path()),
            json!({"stage_aut": false, "fader_links": [{"instance": "band", "id": "REV", "on": false}]})
        );
        let record = records
            .try_iter()
            .find(|r| r["ev"] == "fader_link")
            .unwrap();
        assert_eq!(
            (
                record["instance"].clone(),
                record["id"].clone(),
                record["on"].clone(),
                record["client"].clone(),
                record["peer"].clone()
            ),
            (
                json!("band"),
                json!("REV"),
                json!(false),
                json!(2),
                json!("10.0.0.2")
            )
        );
        // A new page gets it at attach; STAGE AUT's save keeps it.
        assert_eq!(
            hubs(&attach(&mut router, 3)),
            vec![(REV.to_string(), json!(false))]
        );
        router.handle(RouterMsg::SetHub {
            client: 1,
            key: "stage_aut".into(),
            value: json!(true),
        });
        assert_eq!(
            saved(dir.path()),
            json!({"stage_aut": true, "fader_links": [{"instance": "band", "id": "REV", "on": false}]})
        );
        // A link no layout leads, and a value that is no bool: errors,
        // nothing saved.
        let _ = first.take();
        router.handle(RouterMsg::SetHub {
            client: 1,
            key: "fader_link|band|VOX1".into(),
            value: json!(true),
        });
        router.handle(RouterMsg::SetHub {
            client: 1,
            key: REV.into(),
            value: json!("yes"),
        });
        let errors = first
            .take()
            .unwrap_or_default()
            .into_iter()
            .filter(|m| matches!(m, ServerMsg::Error { .. }))
            .count();
        assert_eq!(errors, 2);
        assert_eq!(router.links.values(), vec![(rev(), false)]);
        assert_eq!(
            saved(dir.path())["fader_links"],
            json!([{"instance": "band", "id": "REV", "on": false}])
        );
        // The status lists every leader's switch.
        let (reply, mut answer) = tokio::sync::oneshot::channel();
        router.handle(RouterMsg::Status { reply });
        let status = answer.try_recv().expect("answered at once");
        assert_eq!(
            status
                .fader_links
                .iter()
                .map(|l| (l.instance.as_str(), l.id.as_str(), l.on))
                .collect::<Vec<_>>(),
            vec![("band", "REV", false)]
        );
    }

    #[tokio::test]
    async fn a_switch_set_to_its_value_again_is_told_and_recorded_but_not_saved() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, records) = offline_router(dir.path());
        let page = attach(&mut router, 1);
        router.handle(layout(1, vec![rev()]));
        let _ = page.take();
        router.handle(RouterMsg::SetHub {
            client: 1,
            key: REV.into(),
            value: json!(true),
        });
        assert_eq!(
            hubs(&page),
            vec![(REV.to_string(), json!(true))],
            "told again"
        );
        assert!(
            !dir.path().join("hub-state.json").exists(),
            "the default's value: nothing changed, nothing saved"
        );
        assert_eq!(
            records
                .try_iter()
                .filter(|r| r["ev"] == "fader_link")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn a_switch_saved_keeps_every_other_hub_value_as_loaded() {
        // #130 review M2: the router saves the hub state whole, so a link's
        // save keeps STAGE AUT on, REVERB's keys and AUTOTUNE AUT (#104) as
        // `hub-state.json` had them (PR 4 reads `reverb_low` across a
        // restart).
        let dir = tempfile::tempdir().unwrap();
        let loaded = HubState {
            stage_aut: true,
            reverb: Saved {
                aut: true,
                low: Some(SavedLow {
                    by: fohmixer_proto::client::Low::Stop,
                    faders: vec![SavedFader {
                        instance: "band".into(),
                        label: "Rev ST".into(),
                        back: 0.85,
                        left: Some(0.6),
                    }],
                }),
            },
            fader_links: Vec::new(),
            autotune_aut: true,
        };
        let (mut router, _rx, _records) = offline_router_with(dir.path(), loaded);
        let _page = attach(&mut router, 1);
        router.handle(layout(1, vec![rev()]));
        router.handle(RouterMsg::SetHub {
            client: 1,
            key: REV.into(),
            value: json!(false),
        });
        assert_eq!(
            saved(dir.path()),
            json!({
                "stage_aut": true,
                "reverb_aut": true,
                "reverb_low": {"by": "stop", "faders": [
                    {"instance": "band", "label": "Rev ST", "back": 0.85, "left": 0.6}]},
                "fader_links": [{"instance": "band", "id": "REV", "on": false}],
                "autotune_aut": true
            })
        );
    }
}

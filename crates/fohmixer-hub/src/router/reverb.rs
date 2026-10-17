//! The router's half of REVERB (#130, D18): each layout's reverb faders,
//! the records and the state every page is told (`rules::reverb` decides;
//! from PR 4 also the pages' taps and REVERB AUT).

use fohmixer_proto::client::ServerMsg;
use fohmixer_proto::layout::RuleFader;
use serde_json::Value;

use super::Router;
use crate::outbox::Outbox;
use crate::rules::reverb::{Act, Reverb, Saved, Step, step_line};

/// The router's REVERB.
pub(super) struct ReverbIo {
    pub rule: Reverb,
}

impl ReverbIo {
    pub fn new(saved: &Saved) -> Self {
        Self {
            rule: Reverb::new(saved),
        }
    }
}

/// The `reverb` record's fields of `step` (from PR 4 with the page that
/// caused it).
fn reverb_fields(step: &Step) -> Value {
    serde_json::to_value(step).unwrap_or(Value::Null)
}

impl Router {
    /// A new layout's reverb faders.
    pub(super) fn reverb_layout(&mut self, faders: Vec<RuleFader>) {
        let acts = self.reverb.rule.set_faders(faders);
        self.reverb_apply(acts);
    }

    /// A new page is told REVERB's state.
    pub(super) fn reverb_attach(&self, outbox: &Outbox) {
        outbox.reply(ServerMsg::Reverb(self.reverb.rule.state()));
    }

    /// Carries out the rule's acts.
    pub(super) fn reverb_apply(&self, acts: Vec<Act>) {
        let mut tell = false;
        for act in acts {
            match act {
                Act::Step(step) => {
                    tracing::info!("{}", step_line(&step));
                    self.io.events.record("reverb", reverb_fields(&step));
                }
                Act::Tell => tell = true,
            }
        }
        if tell {
            let state = self.reverb.rule.state();
            for outbox in self.clients.values() {
                outbox.reply(ServerMsg::Reverb(state.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use fohmixer_proto::client::{ReverbState, ServerMsg};
    use fohmixer_proto::layout::RuleFader;
    use serde_json::{Value, json};

    use super::super::RouterMsg;
    use super::super::tests::{attach, offline_router};
    use crate::outbox::Outbox;

    /// The `reverb` messages waiting in `outbox`.
    fn reverb(outbox: &Outbox) -> Vec<ReverbState> {
        outbox
            .take()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|m| match m {
                ServerMsg::Reverb(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn a_page_is_told_the_reverb_faders_at_attach_and_at_a_new_layout() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, records) = offline_router(dir.path());
        let first = attach(&mut router, 1);
        assert_eq!(reverb(&first).len(), 1, "at attach");
        let fader = RuleFader {
            instance: "band".into(),
            label: "Rev ST".into(),
            target: "live_set tracks 4 mixer_device volume".into(),
        };
        router.handle(RouterMsg::Layout {
            rev: 1,
            stage: None,
            targets: BTreeMap::new(),
            strips: Vec::new(),
            reverb: vec![fader],
            fader_leaders: Vec::new(),
        });
        let told = reverb(&first);
        assert_eq!(told.last().map(|s| s.faders.len()), Some(1));
        let steps: Vec<Value> = records.try_iter().filter(|r| r["ev"] == "reverb").collect();
        assert_eq!(steps.last().unwrap()["step"], json!("faders"));
        assert_eq!(steps.last().unwrap()["labels"], json!(["Rev ST"]));
        let second = attach(&mut router, 2);
        assert_eq!(
            reverb(&second).last().map(|s| s
                .faders
                .iter()
                .map(|f| f.label.clone())
                .collect::<Vec<_>>()),
            Some(vec!["Rev ST".to_string()]),
            "a page attached later is told the faders as they are"
        );
    }
}

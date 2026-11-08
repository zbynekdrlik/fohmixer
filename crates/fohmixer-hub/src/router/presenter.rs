//! The router's half of Presenter's stage tab (#90): every page is told the
//! `presenter` message at attach and at each change the link tells
//! (`presenter/link.rs`: up or down, the layouts, the one on stage), only
//! with `[presenter]` (a page without it shows no PRESENTER tab). A page's
//! `presenter_switch` goes to the link's `POST /stage/layout` unless
//! [`switch_refusal`] refuses it; either way the page gets a
//! `presenter_ack` and the switch is one `presenter_switch` record with the
//! page that asked. A page has at most one switch on its way. The layout on
//! stage comes only from the link (Presenter tells every switch on its live
//! socket, the hub's own included), never from a switch's answer, which can
//! arrive after a newer switch's event.

use std::collections::HashSet;
use std::time::Instant;

use fohmixer_proto::client::{ServerMsg, StageLayout};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::{ClientId, Router};
use crate::outbox::Outbox;
use crate::presenter::link::{LinkEvent, MAX_CODE, SwitchRequest};
use crate::ration::{Ration, clip};

/// Warn lines about refused or failed switches per `ration::GAP` (a page in
/// a loop never floods the hub's log); the rest at debug.
pub const SWITCH_LINES: u32 = 8;

/// Presenter's stage as the router tells it, and where switches go.
pub(super) struct PresenterIo {
    online: Option<bool>,
    layouts: Vec<StageLayout>,
    current: Option<String>,
    switches: mpsc::UnboundedSender<SwitchRequest>,
    /// The pages with a switch on its way to Presenter.
    in_flight: HashSet<ClientId>,
    /// The warn lines' ration ([`SWITCH_LINES`]).
    lines: Ration,
}

impl PresenterIo {
    /// The `presenter` message now.
    fn msg(&self) -> ServerMsg {
        ServerMsg::Presenter {
            online: self.online,
            layouts: self.layouts.clone(),
            current: self.current.clone(),
        }
    }
}

/// Why a page's switch to `code` is refused before Presenter is asked
/// (pure): Presenter not up, the page's previous switch still on its way
/// (`busy`), or a code it does not list (or one too long to be any of its).
pub fn switch_refusal(
    online: Option<bool>,
    busy: bool,
    layouts: &[StageLayout],
    code: &str,
) -> Option<String> {
    if online != Some(true) {
        return Some("Presenter can't be reached".to_string());
    }
    if busy {
        return Some("this page's previous switch is still on its way".to_string());
    }
    let listed = code.chars().count() <= MAX_CODE && layouts.iter().any(|l| l.code == code);
    (!listed).then(|| format!("Presenter does not list {:?}", clip(code)))
}

/// The `presenter_switch` record of a switch: the page (`client`, `peer`),
/// the layout asked for and the one on stage before, the outcome and how
/// long Presenter took (none: refused by the hub).
pub fn switch_fields(
    client: ClientId,
    peer: Option<&str>,
    code: &str,
    from: Option<&str>,
    result: &Result<String, String>,
    ms: Option<u64>,
) -> Value {
    let mut fields = json!({
        "client": client, "peer": peer, "code": clip(code), "from": from,
        "ok": result.is_ok(), "ms": ms
    });
    match result {
        Ok(on_stage) => fields["on_stage"] = json!(on_stage),
        Err(why) => fields["error"] = json!(why),
    }
    fields
}

impl Router {
    /// The router of a hub with `[presenter]`: its switches go to
    /// `switches`; the link is not known until the first try decides.
    #[must_use]
    pub fn with_presenter(mut self, switches: mpsc::UnboundedSender<SwitchRequest>) -> Self {
        self.presenter = Some(PresenterIo {
            online: None,
            layouts: Vec::new(),
            current: None,
            switches,
            in_flight: HashSet::new(),
            lines: Ration::default(),
        });
        self
    }

    /// A new client is told of Presenter's stage.
    pub(super) fn presenter_attach(&self, outbox: &Outbox) {
        if let Some(io) = &self.presenter {
            outbox.reply(io.msg());
        }
    }

    /// The link's news: every page is told when it changes what they show.
    pub(super) fn presenter_link(&mut self, event: LinkEvent) {
        let Some(io) = self.presenter.as_mut() else {
            return;
        };
        let changed = match event {
            LinkEvent::Online(online) => {
                let changed = io.online != Some(online);
                io.online = Some(online);
                changed
            }
            LinkEvent::Listed { layouts, current } => {
                let changed = io.layouts != layouts || io.current.as_deref() != Some(&current);
                io.layouts = layouts;
                io.current = Some(current);
                changed
            }
            LinkEvent::Layout { code } => {
                let changed = io.current.as_deref() != Some(&code);
                io.current = Some(code);
                changed
            }
        };
        if changed {
            let msg = io.msg();
            for outbox in self.clients.values() {
                outbox.reply(msg.clone());
            }
        }
    }

    /// A page's held key: to Presenter, or refused at once.
    pub(super) fn presenter_switch(&mut self, client: ClientId, code: String, seq: u64) {
        let Some(io) = self.presenter.as_mut() else {
            return;
        };
        let from = io.current.clone();
        let busy = io.in_flight.contains(&client);
        if let Some(why) = switch_refusal(io.online, busy, &io.layouts, &code) {
            self.presenter_switched(client, seq, (code, from), Err(why), None);
            return;
        }
        let request = SwitchRequest {
            client,
            seq,
            code,
            from,
        };
        match io.switches.send(request) {
            Ok(()) => {
                io.in_flight.insert(client);
            }
            Err(e) => {
                let SwitchRequest { code, from, .. } = e.0;
                let why = "the hub's link to Presenter has stopped".to_string();
                self.presenter_switched(client, seq, (code, from), Err(why), None);
            }
        }
    }

    /// A switch's outcome (`asked`: the code and the layout on stage when
    /// the page asked; `ms`: Presenter's time, none: the hub refused it):
    /// its record and the page's `presenter_ack`; the page may switch again.
    /// The layout on stage is the link's to tell (module doc).
    pub(super) fn presenter_switched(
        &mut self,
        client: ClientId,
        seq: u64,
        asked: (String, Option<String>),
        result: Result<String, String>,
        ms: Option<u64>,
    ) {
        let (code, from) = asked;
        let peer = self.peers.get(&client).map(String::as_str);
        let fields = switch_fields(client, peer, &code, from.as_deref(), &result, ms);
        if ms.is_some()
            && let Some(io) = self.presenter.as_mut()
        {
            io.in_flight.remove(&client);
        }
        match &result {
            Ok(on_stage) => {
                tracing::info!(client, code = ?on_stage, from = ?from, ms, "presenter: switched");
            }
            // Only the refused and failed switches take from the ration.
            Err(why) => match self
                .presenter
                .as_mut()
                .and_then(|io| io.lines.take(Instant::now(), SWITCH_LINES))
            {
                Some(skipped) => tracing::warn!(
                    client,
                    code = ?clip(&code),
                    ms,
                    skipped,
                    "presenter: switch failed: {why}"
                ),
                None => {
                    tracing::debug!(client, code = ?clip(&code), ms, "presenter: switch failed: {why}");
                }
            },
        }
        self.io.events.record("presenter_switch", fields);
        if let Some(outbox) = self.clients.get(&client) {
            outbox.reply(ServerMsg::PresenterAck {
                seq,
                code: code.clone(),
                ok: result.is_ok(),
                error: result.as_ref().err().cloned(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use fohmixer_proto::client::{ServerMsg, StageLayout};
    use tokio::sync::mpsc;

    use super::super::{Router, RouterIo, RouterMsg};
    use super::{SWITCH_LINES, switch_fields, switch_refusal};
    use crate::events::EventLog;
    use crate::outbox::Outbox;
    use crate::presenter::link::{LinkEvent, SwitchRequest};

    fn layout(code: &str) -> StageLayout {
        StageLayout {
            code: code.into(),
            name: code.to_uppercase(),
            description: format!("{code} on stage"),
        }
    }

    struct Bed {
        router: Router,
        switches: mpsc::UnboundedReceiver<SwitchRequest>,
        records: std::sync::mpsc::Receiver<serde_json::Value>,
        _dir: tempfile::TempDir,
    }

    fn bed() -> Bed {
        let dir = tempfile::tempdir().unwrap();
        let (events, records) = EventLog::channel(64);
        let (tx, _) = mpsc::unbounded_channel();
        let (switch_tx, switches) = mpsc::unbounded_channel();
        let router = Router::new(
            BTreeMap::new(),
            crate::rules::HubState::default(),
            dir.path().to_path_buf(),
            RouterIo { tx, events },
        )
        .with_presenter(switch_tx);
        Bed {
            router,
            switches,
            records,
            _dir: dir,
        }
    }

    fn attach(router: &mut Router, client: u64) -> Arc<Outbox> {
        let outbox = Arc::new(Outbox::new());
        router.handle(RouterMsg::Attach {
            client,
            outbox: Arc::clone(&outbox),
            peer: format!("10.0.0.{client}"),
            lan: true,
        });
        outbox
    }

    /// The `presenter` and `presenter_ack` messages waiting in `outbox`.
    fn told(outbox: &Outbox) -> Vec<ServerMsg> {
        let waiting = outbox.take().unwrap_or_default();
        waiting
            .into_iter()
            .filter(|msg| {
                matches!(
                    msg,
                    ServerMsg::Presenter { .. } | ServerMsg::PresenterAck { .. }
                )
            })
            .collect()
    }

    fn stage(online: Option<bool>, layouts: &[&str], current: Option<&str>) -> ServerMsg {
        ServerMsg::Presenter {
            online,
            layouts: layouts.iter().map(|c| layout(c)).collect(),
            current: current.map(str::to_string),
        }
    }

    fn up(router: &mut Router) {
        let listed = LinkEvent::Listed {
            layouts: vec![layout("worship"), layout("timer")],
            current: "worship".into(),
        };
        assert!(router.handle(RouterMsg::PresenterLink { event: listed }));
        assert!(router.handle(RouterMsg::PresenterLink {
            event: LinkEvent::Online(true)
        }));
    }

    fn record(bed: &mut Bed) -> serde_json::Value {
        bed.records.try_recv().expect("a record")
    }

    #[test]
    fn every_page_is_told_of_the_stage_at_attach_and_at_each_change_only() {
        let mut bed = bed();
        let first = attach(&mut bed.router, 1);
        assert_eq!(told(&first), [stage(None, &[], None)], "not known yet");
        up(&mut bed.router);
        assert_eq!(
            told(&first),
            [
                stage(None, &["worship", "timer"], Some("worship")),
                stage(Some(true), &["worship", "timer"], Some("worship")),
            ]
        );
        // The same news again changes nothing, so nothing is sent.
        up(&mut bed.router);
        assert!(told(&first).is_empty());
        let second = attach(&mut bed.router, 2);
        assert_eq!(
            told(&second),
            [stage(Some(true), &["worship", "timer"], Some("worship"))]
        );
        // Anyone's switch in Presenter reaches every page.
        bed.router.handle(RouterMsg::PresenterLink {
            event: LinkEvent::Layout {
                code: "timer".into(),
            },
        });
        let now = stage(Some(true), &["worship", "timer"], Some("timer"));
        assert_eq!(told(&first), std::slice::from_ref(&now));
        assert_eq!(told(&second), [now]);
        // Down keeps what the hub last read.
        bed.router.handle(RouterMsg::PresenterLink {
            event: LinkEvent::Online(false),
        });
        assert_eq!(
            told(&first),
            [stage(Some(false), &["worship", "timer"], Some("timer"))]
        );
    }

    #[test]
    fn a_new_session_tells_the_pages_a_changed_list_or_a_changed_layout_on_stage() {
        let mut bed = bed();
        let page = attach(&mut bed.router, 1);
        up(&mut bed.router);
        told(&page);
        // The same list, another layout on stage (switched while the hub
        // was away): news.
        bed.router.handle(RouterMsg::PresenterLink {
            event: LinkEvent::Listed {
                layouts: vec![layout("worship"), layout("timer")],
                current: "timer".into(),
            },
        });
        assert_eq!(
            told(&page),
            [stage(Some(true), &["worship", "timer"], Some("timer"))]
        );
        // Another list, the same layout on stage: news.
        bed.router.handle(RouterMsg::PresenterLink {
            event: LinkEvent::Listed {
                layouts: vec![layout("timer")],
                current: "timer".into(),
            },
        });
        assert_eq!(told(&page), [stage(Some(true), &["timer"], Some("timer"))]);
    }

    #[test]
    fn a_page_without_presenter_is_told_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (events, _) = EventLog::channel(64);
        let (tx, _) = mpsc::unbounded_channel();
        let mut router = Router::new(
            BTreeMap::new(),
            crate::rules::HubState::default(),
            dir.path().to_path_buf(),
            RouterIo { tx, events },
        );
        let page = attach(&mut router, 1);
        router.handle(RouterMsg::PresenterLink {
            event: LinkEvent::Online(true),
        });
        router.handle(RouterMsg::PresenterSwitch {
            client: 1,
            code: "timer".into(),
            seq: 1,
        });
        assert!(told(&page).is_empty());
    }

    #[test]
    fn a_listed_switch_goes_to_presenter_and_its_outcome_to_the_page_and_the_log() {
        let mut bed = bed();
        let page = attach(&mut bed.router, 3);
        let other = attach(&mut bed.router, 4);
        up(&mut bed.router);
        told(&page);
        told(&other);
        bed.router.handle(RouterMsg::PresenterSwitch {
            client: 3,
            code: "timer".into(),
            seq: 7,
        });
        assert_eq!(
            bed.switches.try_recv().unwrap(),
            SwitchRequest {
                client: 3,
                seq: 7,
                code: "timer".into(),
                from: Some("worship".into()),
            }
        );
        assert!(told(&page).is_empty(), "no answer before Presenter's");
        bed.router.handle(RouterMsg::PresenterSwitched {
            client: 3,
            seq: 7,
            code: "timer".into(),
            from: Some("worship".into()),
            result: Ok("timer".into()),
            ms: 12,
        });
        // The answer is the page's alone: the stage comes from the link.
        assert_eq!(
            told(&page),
            [ServerMsg::PresenterAck {
                seq: 7,
                code: "timer".into(),
                ok: true,
                error: None,
            }]
        );
        assert!(told(&other).is_empty());
        let written = record(&mut bed);
        assert_eq!(written["ev"], "presenter_switch");
        assert_eq!(written["client"], 3);
        assert_eq!(written["peer"], "10.0.0.3");
        assert_eq!(written["code"], "timer");
        assert_eq!(written["from"], "worship");
        assert_eq!(written["ok"], true);
        assert_eq!(written["on_stage"], "timer");
        assert_eq!(written["ms"], 12);
    }

    #[test]
    fn a_switchs_record_names_the_layout_on_stage_when_the_page_asked() {
        let mut bed = bed();
        let page = attach(&mut bed.router, 3);
        up(&mut bed.router);
        bed.router.handle(RouterMsg::PresenterSwitch {
            client: 3,
            code: "timer".into(),
            seq: 9,
        });
        let request = bed.switches.try_recv().unwrap();
        assert_eq!(request.from.as_deref(), Some("worship"));
        // Presenter's own event comes before its answer.
        bed.router.handle(RouterMsg::PresenterLink {
            event: LinkEvent::Layout {
                code: "timer".into(),
            },
        });
        told(&page);
        bed.router.handle(RouterMsg::PresenterSwitched {
            client: request.client,
            seq: request.seq,
            code: request.code,
            from: request.from,
            result: Ok("timer".into()),
            ms: 30,
        });
        assert_eq!(
            told(&page),
            [ServerMsg::PresenterAck {
                seq: 9,
                code: "timer".into(),
                ok: true,
                error: None,
            }],
            "the stage did not change again"
        );
        let written = record(&mut bed);
        assert_eq!(written["from"], "worship");
        assert_eq!(written["on_stage"], "timer");
    }

    #[test]
    fn a_failed_switch_tells_the_page_why_and_changes_no_stage() {
        let mut bed = bed();
        let page = attach(&mut bed.router, 3);
        up(&mut bed.router);
        told(&page);
        bed.router.handle(RouterMsg::PresenterSwitched {
            client: 3,
            seq: 8,
            code: "timer".into(),
            from: Some("worship".into()),
            result: Err("Presenter does not list \"timer\"".into()),
            ms: 9,
        });
        assert_eq!(
            told(&page),
            [ServerMsg::PresenterAck {
                seq: 8,
                code: "timer".into(),
                ok: false,
                error: Some("Presenter does not list \"timer\"".into()),
            }]
        );
        let written = record(&mut bed);
        assert_eq!(written["ok"], false);
        assert_eq!(written["error"], "Presenter does not list \"timer\"");
        assert_eq!(written["from"], "worship");
    }

    #[test]
    fn a_switch_presenter_cannot_take_is_refused_at_once_and_recorded() {
        let mut bed = bed();
        let page = attach(&mut bed.router, 5);
        // Not known yet: refused.
        bed.router.handle(RouterMsg::PresenterSwitch {
            client: 5,
            code: "timer".into(),
            seq: 1,
        });
        up(&mut bed.router);
        // A code Presenter does not list: refused.
        bed.router.handle(RouterMsg::PresenterSwitch {
            client: 5,
            code: "nope".into(),
            seq: 2,
        });
        assert!(
            bed.switches.try_recv().is_err(),
            "nothing went to Presenter"
        );
        let acks: Vec<ServerMsg> = told(&page)
            .into_iter()
            .filter(|m| matches!(m, ServerMsg::PresenterAck { .. }))
            .collect();
        assert_eq!(
            acks,
            [
                ServerMsg::PresenterAck {
                    seq: 1,
                    code: "timer".into(),
                    ok: false,
                    error: Some("Presenter can't be reached".into()),
                },
                ServerMsg::PresenterAck {
                    seq: 2,
                    code: "nope".into(),
                    ok: false,
                    error: Some("Presenter does not list \"nope\"".into()),
                },
            ]
        );
        let first = record(&mut bed);
        assert_eq!(first["ms"], serde_json::Value::Null, "the hub refused it");
        assert_eq!(first["ok"], false);
        assert_eq!(record(&mut bed)["code"], "nope");
    }

    #[test]
    fn a_switch_whose_link_task_stopped_is_answered_failed() {
        let mut bed = bed();
        let page = attach(&mut bed.router, 6);
        up(&mut bed.router);
        told(&page);
        drop(std::mem::replace(
            &mut bed.switches,
            mpsc::unbounded_channel().1,
        ));
        bed.router.handle(RouterMsg::PresenterSwitch {
            client: 6,
            code: "timer".into(),
            seq: 3,
        });
        assert_eq!(
            told(&page),
            [ServerMsg::PresenterAck {
                seq: 3,
                code: "timer".into(),
                ok: false,
                error: Some("the hub's link to Presenter has stopped".into()),
            }]
        );
    }

    #[test]
    fn a_switch_is_refused_unless_presenter_is_up_the_page_free_and_the_code_listed() {
        let layouts = [layout("timer")];
        assert_eq!(switch_refusal(Some(true), false, &layouts, "timer"), None);
        for online in [None, Some(false)] {
            assert_eq!(
                switch_refusal(online, false, &layouts, "timer").as_deref(),
                Some("Presenter can't be reached")
            );
        }
        assert_eq!(
            switch_refusal(Some(true), true, &layouts, "timer").as_deref(),
            Some("this page's previous switch is still on its way")
        );
        assert_eq!(
            switch_refusal(Some(true), false, &layouts, "TIMER").as_deref(),
            Some("Presenter does not list \"TIMER\"")
        );
        // Up to 64 characters may be a code; one more is none.
        let longest = "x".repeat(64);
        let long = [layout(&longest)];
        assert_eq!(switch_refusal(Some(true), false, &long, &longest), None);
        let longer = "x".repeat(65);
        let longer_list = [layout(&longer)];
        assert!(switch_refusal(Some(true), false, &longer_list, &longer).is_some());
        assert_eq!(SWITCH_LINES, 8);
    }

    #[test]
    fn a_page_has_one_switch_on_its_way_and_may_switch_again_after_its_answer() {
        let mut bed = bed();
        let page = attach(&mut bed.router, 3);
        let other = attach(&mut bed.router, 4);
        up(&mut bed.router);
        told(&page);
        let ask = |router: &mut Router, client: u64, seq: u64| {
            router.handle(RouterMsg::PresenterSwitch {
                client,
                code: "timer".into(),
                seq,
            });
        };
        ask(&mut bed.router, 3, 1);
        ask(&mut bed.router, 3, 2);
        ask(&mut bed.router, 4, 1);
        let sent: Vec<(u64, u64)> = std::iter::from_fn(|| bed.switches.try_recv().ok())
            .map(|r| (r.client, r.seq))
            .collect();
        assert_eq!(sent, [(3, 1), (4, 1)], "another page is not held back");
        assert_eq!(
            told(&page),
            [ServerMsg::PresenterAck {
                seq: 2,
                code: "timer".into(),
                ok: false,
                error: Some("this page's previous switch is still on its way".into()),
            }]
        );
        told(&other);
        // A refusal by the hub frees nothing: the first is still on its way.
        ask(&mut bed.router, 3, 3);
        assert!(bed.switches.try_recv().is_err());
        bed.router.handle(RouterMsg::PresenterSwitched {
            client: 3,
            seq: 1,
            code: "timer".into(),
            from: Some("worship".into()),
            result: Err("Presenter does not list \"timer\"".into()),
            ms: 4,
        });
        ask(&mut bed.router, 3, 4);
        assert_eq!(bed.switches.try_recv().unwrap().seq, 4);
    }

    #[test]
    fn a_late_answer_never_puts_back_an_older_layout() {
        let mut bed = bed();
        let page = attach(&mut bed.router, 3);
        up(&mut bed.router);
        bed.router.handle(RouterMsg::PresenterSwitch {
            client: 3,
            code: "timer".into(),
            seq: 1,
        });
        // Presenter tells this switch, then someone else's, then answers.
        for code in ["timer", "worship"] {
            bed.router.handle(RouterMsg::PresenterLink {
                event: LinkEvent::Layout { code: code.into() },
            });
        }
        told(&page);
        bed.router.handle(RouterMsg::PresenterSwitched {
            client: 3,
            seq: 1,
            code: "timer".into(),
            from: Some("worship".into()),
            result: Ok("timer".into()),
            ms: 40,
        });
        assert_eq!(
            told(&page),
            [ServerMsg::PresenterAck {
                seq: 1,
                code: "timer".into(),
                ok: true,
                error: None,
            }],
            "worship stays on stage"
        );
        let late = attach(&mut bed.router, 5);
        assert_eq!(
            told(&late),
            [stage(Some(true), &["worship", "timer"], Some("worship"))]
        );
    }

    #[test]
    fn a_switch_record_names_the_page_the_layouts_and_the_outcome() {
        let ok = switch_fields(
            2,
            Some("10.0.0.2"),
            "timer",
            Some("bible"),
            &Ok("timer".into()),
            Some(5),
        );
        assert_eq!(
            ok,
            serde_json::json!({"client": 2, "peer": "10.0.0.2", "code": "timer",
                "from": "bible", "ok": true, "ms": 5, "on_stage": "timer"})
        );
        let failed = switch_fields(2, None, "x", None, &Err("no".into()), None);
        assert_eq!(
            failed,
            serde_json::json!({"client": 2, "peer": null, "code": "x",
                "from": null, "ok": false, "ms": null, "error": "no"})
        );
    }
}

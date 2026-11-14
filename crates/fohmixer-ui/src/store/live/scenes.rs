//! The scene automation's page side (#207): a strip's rule for a scene set
//! in its channel detail's SCENES, sent with an id from the pending-request
//! counter as the colours' is, so the hub's `result` or `error` reaches its
//! `done`; and a scene the hub tells. The document only ever comes from the
//! hub (`ServerMsg::Scenes`); nothing here reaches Live.

use fohmixer_proto::client::ClientMsg;
use fohmixer_proto::scenes::SceneRule;
use fohmixer_proto::sectors::StripRef;
use leptos::prelude::*;

use super::super::SceneNews;
use super::{LiveStore, ResultFn};
use crate::behave::scenes::{flashes, keeps_last};

impl LiveStore {
    /// Sets `strip`'s rule for `scene` (none: removed). Whether the socket
    /// took it.
    pub fn set_scene_rule(
        self,
        strip: &StripRef,
        scene: &str,
        rule: Option<SceneRule>,
        done: Option<ResultFn>,
    ) -> bool {
        self.sector_request(
            |id| ClientMsg::SetSceneRule {
                id,
                strip: strip.clone(),
                scene: scene.to_string(),
                rule,
            },
            done,
        )
    }

    /// The hub's document: kept, and the last scene forgotten when the
    /// document no longer knows it (`behave::scenes::keeps_last`).
    pub(super) fn on_scenes(self, doc: fohmixer_proto::scenes::ScenesDoc) {
        let forget = self
            .scene
            .try_with(|s| s.as_ref().is_some_and(|n| !keeps_last(&doc, &n.name)))
            .unwrap_or(false);
        if forget {
            let _ = self.scene.try_set(None);
        }
        let _ = self.scenes.try_update(|scenes| scenes.put(doc));
    }

    /// A scene the hub told: the last one kept, and a flash when it came
    /// just now and moved a fader (`behave::scenes::flashes`).
    pub(super) fn on_scene(self, name: String, at_ms: f64, moved: u32, live: bool) {
        let _ = self.scene.try_set(Some(SceneNews { name, at_ms, moved }));
        if flashes(live, moved) {
            let _ = self.scene_flashes.try_update(|n| *n += 1);
        }
    }
}

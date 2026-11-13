//! The loudness parts of the layout (#151, #184): a strip's loudness point
//! (`+LUFS` / `+LUFS:<target>`), a `lufs_readout` of `global` (`STREAM
//! LUFS`), and the served points the hub matches devices to.

use serde::{Deserialize, Serialize};

use super::{Anchor, Binding, Control, Layout, strip_label};

/// The lowest target a `+LUFS:<target>` takes (#151, spec §5.1).
pub const LUFS_TARGET_MIN: i8 = -40;
/// The highest target a `+LUFS:<target>` takes.
pub const LUFS_TARGET_MAX: i8 = -10;

/// A strip's loudness point (#151, spec §3.1, §5.1): with a target an
/// INPUT point (after the trim, its `Voice` judged against the target),
/// without one an OUTPUT point (its plain short-term).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LufsSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<i8>,
}

/// Which loudness a point's number is (#151, the owner's FINDING of 27.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LufsKind {
    /// After the trim, judged against the target: `LUFS IN`.
    In,
    /// An output, plain: `LUFS OUT`.
    Out,
}

impl LufsSpec {
    /// `In` with a target, `Out` without (the kind comes from the tag
    /// alone, never from the track's place).
    pub fn kind(&self) -> LufsKind {
        match self.target {
            Some(_) => LufsKind::In,
            None => LufsKind::Out,
        }
    }
}

/// Whether `target` is a target a loudness point takes.
pub fn lufs_target_ok(target: i8) -> bool {
    (LUFS_TARGET_MIN..=LUFS_TARGET_MAX).contains(&target)
}

/// The longest label a `lufs_readout` takes, in characters (#184).
pub const READOUT_LABEL_MAX: usize = 16;

/// One loudness point of a served layout (#151): its strip's instance,
/// anchor and track target (canonical), its spec and its label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LufsPoint {
    pub instance: String,
    pub anchor: Anchor,
    pub target: String,
    pub spec: LufsSpec,
    pub label: String,
}

impl Layout {
    /// The loudness points (#151, spec §4.4): every strip with `lufs`, each
    /// track once (a strip shows in several views), in document order of
    /// its first place; its target in canonical form (the key a page holds
    /// its strip by), its anchor (the hub names its track from it) and its
    /// label (the strip's own, else its track name's first word).
    ///
    /// Then each `lufs_readout` of `global` (#184) whose track no strip
    /// point names: its target, its label; a strip point on the same track
    /// wins (its spec and label), so both read the one device.
    pub fn lufs_points(&self) -> Vec<LufsPoint> {
        let mut points: Vec<LufsPoint> = Vec::new();
        let strips = self.controls().into_iter().filter_map(|c| match c {
            Control::Strip(strip) => strip.lufs.map(|spec| {
                let label = |target: &str| match (&strip.label, &strip.binding.anchor) {
                    (Some(label), _) => label.clone(),
                    (None, Anchor::Track { name } | Anchor::Return { name }) => strip_label(name),
                    (None, _) => target.to_string(),
                };
                (&strip.binding, spec, label)
            }),
            _ => None,
        });
        let mut add = |binding: &Binding, spec: LufsSpec, label: &dyn Fn(&str) -> String| {
            let track = Binding {
                instance: binding.instance.clone(),
                anchor: binding.anchor.clone(),
                path: None,
            };
            let Ok(target) = track.target() else {
                return;
            };
            let target = crate::path::canonical_target(&target);
            if points
                .iter()
                .any(|p| p.instance == binding.instance && p.target == target)
            {
                return;
            }
            points.push(LufsPoint {
                instance: binding.instance.clone(),
                anchor: binding.anchor.clone(),
                label: label(&target),
                target,
                spec,
            });
        };
        for (binding, spec, label) in strips {
            add(binding, spec, &label);
        }
        for control in readouts(self) {
            if let Control::LufsReadout {
                label,
                target,
                binding,
            } = control
            {
                let spec = LufsSpec {
                    target: Some(*target),
                };
                add(binding, spec, &|_: &str| label.clone());
            }
        }
        points
    }
}

/// The `lufs_readout` controls of `layout`'s `global`, in document order
/// (#184; a readout anywhere else is refused by [`Layout::validate`]).
pub fn readouts(layout: &Layout) -> Vec<&Control> {
    layout
        .global
        .iter()
        .filter(|c| matches!(c, Control::LufsReadout { .. }))
        .collect()
}

/// A `lufs_readout` outside `global` (#184).
pub(super) const OUTSIDE_GLOBAL: &str = "lufs_readout only in global";

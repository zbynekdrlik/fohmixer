//! The channel detail (spec F27, D17; #71 PR D): a hold on a strip's ☰
//! opens that channel over the whole screen.

use fohmixer_proto::layout::Strip;
use leptos::prelude::*;

/// The channel detail's strip, as held (`Nav.detail` on the surface): a
/// context, so a hold on a strip's ☰ opens it and its exit closes it.
#[derive(Clone, Copy)]
pub struct DetailStrip(pub RwSignal<Option<Strip>>);

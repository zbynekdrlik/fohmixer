//! Static items (spec F1, F19): area boxes with their titles, and texts.

use fohmixer_proto::layout::{Frame, Style};
use leptos::prelude::*;

use crate::stage;

/// A background box, optionally titled (vertically when the style says).
#[component]
pub fn AreaView(frame: Frame, z: i64, style: Style, title: Option<String>) -> impl IntoView {
    let css = stage::item_style(frame, z, &style);
    let text = title.or(style.text).unwrap_or_default();
    let class = if style.vertical {
        "item area vertical"
    } else {
        "item area"
    };
    view! {
        <div class=class data-testid="area" style=css>
            <span class="area-title">{text}</span>
        </div>
    }
}

/// A static text (the Conf text keeps its lines).
#[component]
pub fn LabelView(frame: Frame, z: i64, style: Style, text: String) -> impl IntoView {
    let css = stage::item_style(frame, z, &style);
    view! {
        <div class="item label" data-testid="label" style=css>
            {text}
        </div>
    }
}

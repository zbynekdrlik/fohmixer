//! The engineer's PIN login (S4 design note §6; the iemmixer engineer
//! login @ 22372bc): four digits submit at once; the token is kept on the
//! device (spec X8).

use leptos::html;
use leptos::prelude::*;

use crate::app::version_text;
use crate::auth::{self, PIN_LEN};

/// The keypad's keys, in order.
const KEYS: [&str; 12] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "CLR", "0", "⌫"];

/// The login page; a successful login sets `session`.
#[component]
pub fn Login(session: RwSignal<Option<String>>) -> impl IntoView {
    let pin = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let root = NodeRef::<html::Div>::new();

    let press = move |key: &str| {
        if busy.try_get_untracked().unwrap_or(true) {
            return;
        }
        let _ = error.try_set(None);
        match key {
            "CLR" => {
                let _ = pin.try_set(String::new());
                return;
            }
            "⌫" => {
                let _ = pin.try_update(|p| {
                    p.pop();
                });
                return;
            }
            _ => {}
        }
        let Some(current) = pin.try_update(|p| {
            if p.len() < PIN_LEN {
                p.push_str(key);
            }
            p.clone()
        }) else {
            return;
        };
        if current.len() != PIN_LEN {
            return;
        }
        let _ = busy.try_set(true);
        leptos::task::spawn_local(async move {
            match auth::login(&current).await {
                Ok(token) => {
                    let _ = session.try_set(Some(token));
                }
                Err(e) => {
                    let _ = error.try_set(Some(e.message()));
                    let _ = pin.try_set(String::new());
                    let _ = busy.try_set(false);
                }
            }
        });
    };
    let on_key = move |ev: web_sys::KeyboardEvent| {
        let key = ev.key();
        let mapped = match key.as_str() {
            "Backspace" => Some("⌫"),
            "Escape" | "Delete" => Some("CLR"),
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()) => {
                KEYS.iter().copied().find(|d| *d == k)
            }
            _ => None,
        };
        if let Some(mapped) = mapped {
            ev.prevent_default();
            press(mapped);
        }
    };
    root.on_load(|el: web_sys::HtmlDivElement| {
        let _ = el.focus();
    });

    let dots = move || {
        let filled = pin.with(String::len);
        let failed = error.with(Option::is_some);
        (0..PIN_LEN)
            .map(|i| {
                let class = if failed {
                    "pin-dot error"
                } else if i < filled {
                    "pin-dot filled"
                } else {
                    "pin-dot"
                };
                view! { <span class=class></span> }
            })
            .collect_view()
    };
    let keys = KEYS
        .into_iter()
        .map(|key: &'static str| {
            view! {
                <button
                    type="button"
                    class="pin-key"
                    data-testid="pin-key"
                    data-key=key
                    disabled=move || busy.get()
                    on:click=move |_| press(key)
                >
                    {key}
                </button>
            }
        })
        .collect_view();
    let message = move || {
        error.get().map(|e| {
            view! { <p class="login-error" data-testid="login-error">{e}</p> }
        })
    };
    view! {
        <div class="login" data-testid="login" tabindex="-1" node_ref=root on:keydown=on_key>
            <div class="login-box">
                <h1 class="login-title">"fohmixer"</h1>
                <p class="login-hint">"Engineer PIN"</p>
                <div class="pin-dots" data-testid="pin-dots">{dots}</div>
                <div class="pin-pad">{keys}</div>
                {message}
            </div>
            <span class="login-version" data-testid="version">{version_text()}</span>
        </div>
    }
}

//! The login line and LOG OUT at the column's foot (#184): which login the
//! tablet holds, where that matters (`behave::login::login_line`), and the
//! way back to the login. The FOH engineer's page shows neither.

use leptos::prelude::*;

use super::owns_touches;
use crate::behave::login::{login_line, one_instance};
use crate::store::LiveStore;

/// The line and its LOG OUT, while the login is worth a line.
#[component]
pub fn LoginLine() -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let line = Memo::new(move |_| {
        // No layout served yet: not one Live set's page (the FOH page
        // without its layout shows no line).
        let one = store
            .layout
            .try_with(|l| l.as_deref().is_some_and(one_instance))
            .unwrap_or(false);
        login_line(store.role.try_get().flatten(), one)
    });
    move || {
        line.try_get().flatten().map(|text| {
            // LOG OUT owns its touches (#43 PR G) and acts on its lift, as
            // AbleSet's sign-in: a finger that slides off does nothing.
            let no_keys: Vec<String> = Vec::new();
            view! {
                <div class="login-line" data-testid="login-line">{text}</div>
                <button
                    type="button"
                    class="logout"
                    use:owns_touches=no_keys
                    data-testid="logout"
                    on:pointerup=move |_| store.log_out()
                >
                    "LOG OUT"
                </button>
            }
        })
    }
}

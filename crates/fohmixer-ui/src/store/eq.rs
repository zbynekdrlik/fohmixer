//! The Pro-Q 4 screen in the store (#71 PR E, F28), pure: the latest
//! `eq_list` answer ([`EqListView`], numbered so a card fetches its picture
//! again on each one), this page's editor ([`EqView`], from the hub's `eq`;
//! `opening` from the moment the page asks), when the screen showing an
//! editor ends ([`ended`]) and what a closed socket does to it
//! ([`socket_closed`]: the hub closes the editor of a socket it lost).
//! `store/live/eq.rs` carries them out.

use fohmixer_proto::eq::{EqItem, EqState};
use fohmixer_proto::layout::Binding;

/// Why the page's editor ended with its socket.
pub const SOCKET: &str = "socket";

/// The latest `eq_list` answer, and its number.
#[derive(Debug, Clone, PartialEq)]
pub struct EqListView {
    pub binding: Binding,
    pub items: Vec<EqItem>,
    pub error: Option<String>,
    pub generation: u64,
}

/// This page's editor.
#[derive(Debug, Clone, PartialEq)]
pub struct EqView {
    pub instance: String,
    pub path: String,
    pub state: EqState,
    pub session: Option<u32>,
    pub size: Option<(u32, u32)>,
    pub reason: Option<String>,
    pub since: Option<f64>,
}

impl EqView {
    /// The page asked to open the editor at `path` on `instance`.
    pub fn opening(instance: &str, path: &str) -> Self {
        Self {
            instance: instance.to_string(),
            path: path.to_string(),
            state: EqState::Opening,
            session: None,
            size: None,
            reason: None,
            since: None,
        }
    }

    /// Whether it is the editor at `path` on `instance`.
    pub fn is(&self, instance: &str, path: &str) -> bool {
        self.instance == instance && self.path == path
    }

    /// Its session while it is open.
    pub fn open_session(&self) -> Option<u32> {
        self.session.filter(|_| self.state == EqState::Open)
    }
}

/// The page's editor right after it asked to open `path` on `instance`
/// (`sent`: the socket took the ask).
pub fn asked(instance: &str, path: &str, _sent: bool) -> EqView {
    EqView::opening(instance, path)
}

/// The page's editor as the screen showing the editor at `path` on
/// `instance` reads it: only when it is that editor. During a switch the
/// hub's late word about the editor before (its close) is not this
/// screen's state, session or size.
pub fn screen_view<'a>(view: Option<&'a EqView>, instance: &str, path: &str) -> Option<&'a EqView> {
    view.filter(|v| v.is(instance, path))
}

/// The screen's state (its `data-state`): its editor's, and `opening`
/// until the hub speaks of that editor (the page asked to open it), so
/// "Otváram Pro-Q 4…" shows until it is open.
pub fn screen_state(view: Option<&EqView>, instance: &str, path: &str) -> &'static str {
    match screen_view(view, instance, path).map(|v| v.state) {
        Some(EqState::Open) => "open",
        Some(EqState::Closed) => "closed",
        Some(EqState::Opening) | None => "opening",
    }
}

/// Why the screen showing the editor at `path` on `instance` ends: the
/// hub's reason once it closed that editor; none while it opens or is open
/// (or the page's editor is another one).
pub fn ended(view: Option<&EqView>, instance: &str, path: &str) -> Option<String> {
    let view = screen_view(view, instance, path)?;
    (view.state == EqState::Closed).then(|| view.reason.clone().unwrap_or_default())
}

/// The page's editor after its socket closed: an editor opening or open is
/// closed (the hub closed it with the socket), why [`SOCKET`].
pub fn socket_closed(view: Option<EqView>) -> Option<EqView> {
    view.map(|v| {
        if v.state == EqState::Closed {
            return v;
        }
        EqView {
            instance: v.instance,
            path: v.path,
            state: EqState::Closed,
            session: None,
            size: v.size,
            reason: Some(SOCKET.to_string()),
            since: v.since,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(path: &str) -> EqView {
        EqView {
            instance: "band".into(),
            path: path.into(),
            state: EqState::Open,
            session: Some(3),
            size: Some((1349, 809)),
            reason: None,
            since: None,
        }
    }

    #[test]
    fn an_editor_opens_from_the_pages_ask() {
        let view = EqView::opening("band", "p");
        assert_eq!(
            view,
            EqView {
                instance: "band".into(),
                path: "p".into(),
                state: EqState::Opening,
                session: None,
                size: None,
                reason: None,
                since: None,
            }
        );
        assert!(view.is("band", "p"));
        assert!(!view.is("master", "p"));
        assert!(!view.is("band", "q"));
        assert_eq!(view.open_session(), None);
        assert_eq!(open("p").open_session(), Some(3));
        let mut closed = open("p");
        closed.state = EqState::Closed;
        assert_eq!(closed.open_session(), None);
    }

    #[test]
    fn the_screen_ends_when_the_hub_closes_its_editor() {
        let opening = EqView::opening("band", "p");
        assert_eq!(ended(Some(&opening), "band", "p"), None);
        assert_eq!(ended(Some(&open("p")), "band", "p"), None);
        assert_eq!(ended(None, "band", "p"), None);
        let mut closed = open("p");
        closed.state = EqState::Closed;
        closed.reason = Some("locked".into());
        assert_eq!(ended(Some(&closed), "band", "p"), Some("locked".into()));
        assert_eq!(ended(Some(&closed), "band", "q"), None, "another editor");
        closed.reason = None;
        assert_eq!(ended(Some(&closed), "band", "p"), Some(String::new()));
    }

    #[test]
    fn an_ask_the_socket_did_not_take_closes_at_once() {
        assert_eq!(asked("band", "p", true), EqView::opening("band", "p"));
        assert_eq!(
            asked("band", "p", false),
            EqView {
                instance: "band".into(),
                path: "p".into(),
                state: EqState::Closed,
                session: None,
                size: None,
                reason: Some("socket".into()),
                since: None,
            }
        );
        // So the screen ends at once (quietly, as any lost socket).
        assert_eq!(
            ended(Some(&asked("band", "p", false)), "band", "p"),
            Some("socket".into())
        );
        assert_eq!(ended(Some(&asked("band", "p", true)), "band", "p"), None);
    }

    #[test]
    fn a_screen_reads_only_its_own_editor() {
        let mut closed = open("q");
        closed.state = EqState::Closed;
        closed.reason = Some("switch".into());
        // A late close of the editor before (a switch from q to p): the
        // screen on p is still opening, with no session.
        assert_eq!(screen_view(Some(&closed), "band", "p"), None);
        assert_eq!(screen_state(Some(&closed), "band", "p"), "opening");
        assert_eq!(screen_view(Some(&open("q")), "band", "p"), None);
        assert_eq!(screen_state(Some(&open("q")), "band", "p"), "opening");
        assert_eq!(screen_view(Some(&open("p")), "master", "p"), None);
        assert_eq!(screen_state(Some(&open("p")), "master", "p"), "opening");
        assert_eq!(screen_state(None, "band", "p"), "opening");
        // Its own editor, in each state.
        assert_eq!(screen_view(Some(&open("p")), "band", "p"), Some(&open("p")));
        assert_eq!(screen_state(Some(&open("p")), "band", "p"), "open");
        let opening = EqView::opening("band", "p");
        assert_eq!(screen_state(Some(&opening), "band", "p"), "opening");
        closed.path = "p".into();
        assert_eq!(screen_state(Some(&closed), "band", "p"), "closed");
    }

    #[test]
    fn a_closed_socket_closes_the_pages_editor() {
        assert_eq!(socket_closed(None), None);
        let closed = socket_closed(Some(open("p"))).unwrap();
        assert_eq!(
            (closed.state, closed.session, closed.reason.as_deref()),
            (EqState::Closed, None, Some("socket"))
        );
        assert_eq!(closed.size, Some((1349, 809)));
        let opening = socket_closed(Some(EqView::opening("band", "p"))).unwrap();
        assert_eq!(opening.reason.as_deref(), Some("socket"));
        // One closed already keeps its reason.
        let mut exit = open("p");
        exit.state = EqState::Closed;
        exit.reason = Some("exit".into());
        assert_eq!(socket_closed(Some(exit.clone())), Some(exit));
    }
}

//! AbleSet's tab (#90, design §5; the owner's variant A of
//! `docs/mockups/ableset-tab-v1.html`), pure: what the hub said of AbleSet's
//! page, what the tab's page shows ([`show`]: AbleSet's own page in a frame,
//! or a panel), the probe that comes first ([`Probe`]: on the internet path
//! AbleSet's name has an Access application of its own, and a frame that
//! Access redirects to its login would show nothing; on the LAN a failed
//! probe says why, #90 PR C review M2), the addresses the page
//! builds from its own (the frame, the probe, the sign-in, AbleSet directly
//! on the PC) and the panels' texts (the mockup's). `pages/ableset.rs`
//! carries it out.

use std::net::IpAddr;

/// AbleSet's page as the hub's latest `ableset` message described it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbleSetInfo {
    /// The tab's title.
    pub title: String,
    /// AbleSet's name (`[ableset] name`).
    pub name: String,
    /// fohmixer's name (`[tls] name`): the only page that frames AbleSet.
    pub parent: String,
    /// The hub's link to AbleSet: `None` until its first read decided.
    pub online: Option<bool>,
    /// AbleSet's own port on the PC (the direct address).
    pub lan_port: u16,
    /// This page's socket came over the LAN (the hub's word, #90 PR C
    /// review M2): a failed probe is no sign-in here.
    pub lan: bool,
}

/// A panel in the frame's place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    /// AbleSet does not answer the hub (or the hub this page, before the
    /// frame was mounted).
    Down,
    /// The probe failed through the tunnel: outside the church, AbleSet
    /// needs its own sign-in.
    SignIn,
    /// The probe failed on the LAN (#90 PR C review M2): AbleSet's name or
    /// certificate does not work here, or it did not answer in time. Not in
    /// the approved mockup.
    Unreachable(Fail),
    /// The page is not fohmixer's own https page (the emergency address).
    Elsewhere,
    /// Not known yet, or the probe on its way.
    Checking,
}

impl Panel {
    /// The panel's name (`data-state`, the flight recorder).
    pub fn name(self) -> &'static str {
        match self {
            Panel::Down => "down",
            Panel::SignIn => "signin",
            Panel::Unreachable(_) => "unreachable",
            Panel::Elsewhere => "elsewhere",
            Panel::Checking => "checking",
        }
    }
}

/// What the tab's page shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Show {
    /// AbleSet's own page in the frame.
    Frame,
    /// A panel instead.
    Panel(Panel),
}

impl Show {
    /// `frame` or the panel's name (`data-state`).
    pub fn name(self) -> &'static str {
        match self {
            Show::Frame => "frame",
            Show::Panel(panel) => panel.name(),
        }
    }
}

/// Why a probe failed: its image's error (a name, a certificate, or the
/// tunnel's Access refusing it), or no answer within [`PROBE_MS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fail {
    Error,
    Late,
}

/// The probe of AbleSet's name (an image the hub serves after its access
/// decision): none yet, the `n`th on its way, loaded, or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Idle,
    Pending(u32),
    Passed,
    Failed(Fail),
}

/// How long a probe may take before it counts as failed (ms).
pub const PROBE_MS: u32 = 8_000;

/// What the tab's page shows: nothing loads unless the page is fohmixer's
/// own https page (`here`); AbleSet down is the down panel (its frame
/// removed); a frame the probe let in stays (also while the page's own
/// socket is down, or a restarted hub has not read AbleSet yet: AbleSet
/// reconnects through the hub by itself); otherwise the page's socket down
/// is the down panel, a link not read yet (`online` none) is checking, never
/// down, and a failed probe is the sign-in through the tunnel, and on the
/// LAN (`lan`, the hub's word for this page) why it failed: never a sign-in
/// that takes the mixer away.
pub fn show(online: Option<bool>, here: bool, connected: bool, probe: Probe, lan: bool) -> Show {
    if !here {
        return Show::Panel(Panel::Elsewhere);
    }
    if online == Some(false) {
        return Show::Panel(Panel::Down);
    }
    if probe == Probe::Passed {
        return Show::Frame;
    }
    if !connected {
        return Show::Panel(Panel::Down);
    }
    if online.is_none() {
        return Show::Panel(Panel::Checking);
    }
    match probe {
        Probe::Failed(why) if lan => Show::Panel(Panel::Unreachable(why)),
        Probe::Failed(_) => Show::Panel(Panel::SignIn),
        _ => Show::Panel(Panel::Checking),
    }
}

/// Whether a probe starts now: none yet, on fohmixer's own page, connected,
/// AbleSet up.
pub fn probes(online: Option<bool>, here: bool, connected: bool, probe: Probe) -> bool {
    probe == Probe::Idle && here && connected && online == Some(true)
}

/// The probe after the link's news: AbleSet down forgets it, so its return
/// probes again and mounts a fresh frame.
pub fn after_link(online: Option<bool>, probe: Probe) -> Probe {
    if online == Some(false) {
        Probe::Idle
    } else {
        probe
    }
}

/// The probe after probe `n`'s answer (loaded, or why not): only the probe
/// on its way takes it (an older one's late answer changes nothing).
pub fn probed(probe: Probe, n: u32, answer: Result<(), Fail>) -> Probe {
    match probe {
        Probe::Pending(m) if m == n => match answer {
            Ok(()) => Probe::Passed,
            Err(why) => Probe::Failed(why),
        },
        other => other,
    }
}

/// The probe when the page comes back visible: a failed one tries again
/// (the sign-in may have happened meanwhile).
pub fn again(probe: Probe) -> Probe {
    match probe {
        Probe::Failed(_) => Probe::Idle,
        other => other,
    }
}

/// The tab's red dot: AbleSet down, or the hub's socket.
pub fn tab_dot(online: Option<bool>, connected: bool) -> bool {
    !connected || online == Some(false)
}

/// Whether the page is fohmixer's own https page (`protocol` and
/// `hostname` as `location` gives them), the only one AbleSet lets frame it.
pub fn framed_here(protocol: &str, hostname: &str, parent: &str) -> bool {
    protocol == "https:" && hostname.eq_ignore_ascii_case(parent)
}

/// AbleSet's origin with the page's own port (`location.port`: empty for
/// the default, as through the tunnel).
fn origin(name: &str, port: &str) -> String {
    if port.is_empty() {
        format!("https://{name}")
    } else {
        format!("https://{name}:{port}")
    }
}

/// The frame's address: AbleSet's page.
pub fn frame_url(name: &str, port: &str) -> String {
    format!("{}/", origin(name, port))
}

/// The probe's address (`n` makes every probe a new request).
pub fn probe_url(name: &str, port: &str, n: u64) -> String {
    format!("{}/.fohmixer/probe?{n}", origin(name, port))
}

/// The sign-in's address: Access signs in top-level, and the hub sends the
/// page back to fohmixer's `#ableset`.
pub fn signin_url(name: &str, port: &str) -> String {
    format!("{}/.fohmixer/signed-in", origin(name, port))
}

/// AbleSet directly on the PC (the emergency path): only when the page was
/// opened by an address or `localhost`.
pub fn direct_url(hostname: &str, lan_port: u16) -> Option<String> {
    let bare = hostname.trim_start_matches('[').trim_end_matches(']');
    let direct = hostname.eq_ignore_ascii_case("localhost") || bare.parse::<IpAddr>().is_ok();
    direct.then(|| format!("http://{hostname}:{lan_port}/"))
}

/// The hash AbleSet's sign-in comes back with: the tab opens once.
pub fn returns_to_tab(hash: &str) -> bool {
    hash == "#ableset"
}

/// A panel's title (none for the brief check).
pub fn panel_title(panel: Panel) -> &'static str {
    match panel {
        Panel::Down => "AbleSet nebeží",
        Panel::SignIn => "Prihlásenie do AbleSetu",
        Panel::Unreachable(_) => "AbleSet sa nedá načítať",
        Panel::Elsewhere => "Núdzová adresa",
        Panel::Checking => "",
    }
}

/// A panel's text; the elsewhere panel's names AbleSet's direct address
/// after it only when there is one (`direct`).
pub fn panel_text(panel: Panel, direct: bool) -> &'static str {
    match panel {
        Panel::Down => {
            "AbleSet na PC teraz neodpovedá. Fohmixer ho skúša znova sám; mixér funguje ďalej."
        }
        Panel::SignIn => {
            "Si mimo kostola. AbleSet chce raz potvrdiť prihlásenie, rovnako ako fohmixer."
        }
        Panel::Unreachable(Fail::Error) => {
            "AbleSet sa cez sieť kostola nenačítal: jeho meno alebo certifikát nefunguje. Mixér funguje ďalej; fohmixer to skúsi znova, keď sa sem vrátiš."
        }
        Panel::Unreachable(Fail::Late) => {
            "AbleSet neodpovedal do 8 sekúnd. Mixér funguje ďalej; fohmixer to skúsi znova, keď sa sem vrátiš."
        }
        Panel::Elsewhere if direct => {
            "Fohmixer je otvorený cez IP bez certifikátu, tak sa AbleSet nedá vložiť. Otvor fohmixer cez jeho meno, alebo AbleSet priamo:"
        }
        Panel::Elsewhere => {
            "Fohmixer nie je otvorený cez svoje meno, tak sa AbleSet nedá vložiť. Otvor fohmixer cez jeho meno."
        }
        Panel::Checking => "Pripájam AbleSet…",
    }
}

/// The sign-in panel's button.
pub const SIGNIN_BUTTON: &str = "PRIHLÁSIŤ SA";

#[cfg(test)]
mod tests;

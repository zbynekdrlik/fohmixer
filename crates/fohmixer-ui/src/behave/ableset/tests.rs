use super::*;

const PANELS: [Panel; 4] = [
    Panel::Down,
    Panel::SignIn,
    Panel::Elsewhere,
    Panel::Checking,
];

#[test]
fn only_fohmixers_own_https_page_frames_ableset() {
    // Elsewhere (the emergency address, the tray's localhost, an address
    // past a certificate warning) loads nothing, whatever the rest says.
    for online in [None, Some(true), Some(false)] {
        for connected in [true, false] {
            for probe in [Probe::Idle, Probe::Pending(1), Probe::Passed, Probe::Failed] {
                assert_eq!(
                    show(online, false, connected, probe),
                    Show::Panel(Panel::Elsewhere),
                    "{online:?} {connected} {probe:?}"
                );
            }
        }
    }
    assert_eq!(show(Some(true), true, true, Probe::Passed), Show::Frame);
}

#[test]
fn ableset_down_removes_the_frame_and_its_return_probes_again() {
    assert_eq!(
        show(Some(false), true, true, Probe::Passed),
        Show::Panel(Panel::Down)
    );
    assert_eq!(
        show(Some(false), true, true, Probe::Idle),
        Show::Panel(Panel::Down)
    );
    assert_eq!(
        show(Some(false), true, false, Probe::Failed),
        Show::Panel(Panel::Down)
    );
    // Down forgets the probe: back up, it probes again and a fresh frame mounts.
    assert_eq!(after_link(Some(false), Probe::Passed), Probe::Idle);
    assert_eq!(after_link(Some(false), Probe::Pending(3)), Probe::Idle);
    assert_eq!(after_link(Some(true), Probe::Passed), Probe::Passed);
    assert_eq!(after_link(None, Probe::Failed), Probe::Failed);
    assert_eq!(
        show(Some(true), true, true, Probe::Idle),
        Show::Panel(Panel::Checking)
    );
    assert!(probes(Some(true), true, true, Probe::Idle));
}

#[test]
fn a_link_not_read_yet_is_checking_never_down() {
    assert_eq!(
        show(None, true, true, Probe::Idle),
        Show::Panel(Panel::Checking)
    );
    assert_eq!(
        show(None, true, true, Probe::Pending(1)),
        Show::Panel(Panel::Checking)
    );
    // A failed probe before a restarted hub's first read: still checking.
    assert_eq!(
        show(None, true, true, Probe::Failed),
        Show::Panel(Panel::Checking)
    );
    // Nothing probes before the hub knows AbleSet is up.
    assert!(!probes(None, true, true, Probe::Idle));
}

#[test]
fn a_mounted_frame_stays_through_the_pages_own_outage_and_a_restarted_hub() {
    assert_eq!(show(Some(true), true, false, Probe::Passed), Show::Frame);
    assert_eq!(show(None, true, false, Probe::Passed), Show::Frame);
    assert_eq!(show(None, true, true, Probe::Passed), Show::Frame);
    // Not mounted yet: the socket down is the down panel.
    assert_eq!(
        show(Some(true), true, false, Probe::Idle),
        Show::Panel(Panel::Down)
    );
    assert_eq!(
        show(Some(true), true, false, Probe::Pending(2)),
        Show::Panel(Panel::Down)
    );
    assert_eq!(
        show(None, true, false, Probe::Failed),
        Show::Panel(Panel::Down)
    );
}

#[test]
fn the_probe_decides_between_the_frame_and_the_sign_in() {
    assert_eq!(
        show(Some(true), true, true, Probe::Pending(1)),
        Show::Panel(Panel::Checking)
    );
    assert_eq!(
        show(Some(true), true, true, Probe::Failed),
        Show::Panel(Panel::SignIn)
    );
    assert_eq!(show(Some(true), true, true, Probe::Passed), Show::Frame);
}

#[test]
fn a_probe_starts_once_on_fohmixers_page_connected_with_ableset_up() {
    assert!(probes(Some(true), true, true, Probe::Idle));
    assert!(!probes(Some(false), true, true, Probe::Idle));
    assert!(!probes(Some(true), false, true, Probe::Idle));
    assert!(!probes(Some(true), true, false, Probe::Idle));
    for probe in [Probe::Pending(1), Probe::Passed, Probe::Failed] {
        assert!(!probes(Some(true), true, true, probe), "{probe:?}");
    }
}

#[test]
fn only_the_probe_on_its_way_takes_an_answer() {
    assert_eq!(probed(Probe::Pending(2), 2, true), Probe::Passed);
    assert_eq!(probed(Probe::Pending(2), 2, false), Probe::Failed);
    // An older probe's late answer (its 8 s ran out, or AbleSet went down
    // meanwhile) changes nothing.
    assert_eq!(probed(Probe::Pending(3), 2, true), Probe::Pending(3));
    assert_eq!(probed(Probe::Pending(3), 2, false), Probe::Pending(3));
    assert_eq!(probed(Probe::Idle, 2, true), Probe::Idle);
    assert_eq!(probed(Probe::Failed, 2, true), Probe::Failed);
    assert_eq!(probed(Probe::Passed, 2, false), Probe::Passed);
}

#[test]
fn a_failed_probe_tries_again_when_the_page_comes_back() {
    assert_eq!(again(Probe::Failed), Probe::Idle);
    assert_eq!(again(Probe::Passed), Probe::Passed);
    assert_eq!(again(Probe::Pending(4)), Probe::Pending(4));
    assert_eq!(again(Probe::Idle), Probe::Idle);
}

#[test]
fn the_tabs_dot_is_red_while_ableset_or_the_hub_is_away() {
    assert!(!tab_dot(Some(true), true));
    assert!(!tab_dot(None, true));
    assert!(tab_dot(Some(false), true));
    assert!(tab_dot(Some(true), false));
    assert!(tab_dot(None, false));
}

#[test]
fn the_page_is_fohmixers_own_only_over_https_by_its_name() {
    assert!(framed_here("https:", "foh.example.org", "foh.example.org"));
    assert!(framed_here("https:", "FOH.example.org", "foh.example.org"));
    assert!(!framed_here("http:", "foh.example.org", "foh.example.org"));
    assert!(!framed_here("https:", "192.0.2.10", "foh.example.org"));
    assert!(!framed_here(
        "https:",
        "ableset.example.org",
        "foh.example.org"
    ));
}

#[test]
fn the_addresses_take_the_pages_own_port() {
    // The LAN: the hub's [tls] port; the tunnel: Cloudflare's 443 (none).
    assert_eq!(
        frame_url("ableset.example.org", "8443"),
        "https://ableset.example.org:8443/"
    );
    assert_eq!(
        frame_url("ableset.example.org", ""),
        "https://ableset.example.org/"
    );
    assert_eq!(
        probe_url("ableset.example.org", "8443", 1_760_000_000_123),
        "https://ableset.example.org:8443/.fohmixer/probe?1760000000123"
    );
    assert_eq!(
        probe_url("ableset.example.org", "", 7),
        "https://ableset.example.org/.fohmixer/probe?7"
    );
    assert_eq!(
        signin_url("ableset.example.org", ""),
        "https://ableset.example.org/.fohmixer/signed-in"
    );
    assert_eq!(
        signin_url("ableset.example.org", "8443"),
        "https://ableset.example.org:8443/.fohmixer/signed-in"
    );
}

#[test]
fn ableset_directly_only_from_an_address_or_localhost() {
    assert_eq!(
        direct_url("192.0.2.10", 80),
        Some("http://192.0.2.10:80/".to_string())
    );
    assert_eq!(
        direct_url("localhost", 8081),
        Some("http://localhost:8081/".to_string())
    );
    assert_eq!(
        direct_url("LOCALHOST", 80),
        Some("http://LOCALHOST:80/".to_string())
    );
    assert_eq!(
        direct_url("[::1]", 80),
        Some("http://[::1]:80/".to_string())
    );
    assert_eq!(direct_url("foh.example.org", 80), None);
    assert_eq!(direct_url("localhost.example.org", 80), None);
    assert_eq!(direct_url("", 80), None);
}

#[test]
fn the_sign_in_returns_to_the_tab_by_its_hash() {
    assert!(returns_to_tab("#ableset"));
    assert!(!returns_to_tab(""));
    assert!(!returns_to_tab("#ableset2"));
    assert!(!returns_to_tab("#"));
}

#[test]
fn the_page_says_what_it_shows() {
    let names: Vec<&str> = PANELS.iter().map(|p| p.name()).collect();
    assert_eq!(names, ["down", "signin", "elsewhere", "checking"]);
    assert_eq!(Show::Frame.name(), "frame");
    assert_eq!(Show::Panel(Panel::SignIn).name(), "signin");
    assert_eq!(Show::Panel(Panel::Down).name(), "down");
}

#[test]
fn the_panels_say_what_the_approved_mockup_says() {
    assert_eq!(panel_title(Panel::Down), "AbleSet nebeží");
    assert_eq!(panel_title(Panel::SignIn), "Prihlásenie do AbleSetu");
    assert_eq!(panel_title(Panel::Elsewhere), "Núdzová adresa");
    assert_eq!(panel_title(Panel::Checking), "");
    assert_eq!(
        panel_text(Panel::Down, true),
        "AbleSet na PC teraz neodpovedá. Fohmixer ho skúša znova sám; mixér funguje ďalej."
    );
    assert_eq!(
        panel_text(Panel::SignIn, false),
        "Si mimo kostola. AbleSet chce raz potvrdiť prihlásenie, rovnako ako fohmixer."
    );
    assert_eq!(
        panel_text(Panel::Elsewhere, true),
        "Fohmixer je otvorený cez IP bez certifikátu, tak sa AbleSet nedá vložiť. Otvor fohmixer cez jeho meno, alebo AbleSet priamo:"
    );
    assert_eq!(
        panel_text(Panel::Elsewhere, false),
        "Fohmixer nie je otvorený cez svoje meno, tak sa AbleSet nedá vložiť. Otvor fohmixer cez jeho meno."
    );
    assert_eq!(panel_text(Panel::Checking, false), "Pripájam AbleSet…");
    assert_eq!(SIGNIN_BUTTON, "PRIHLÁSIŤ SA");
}

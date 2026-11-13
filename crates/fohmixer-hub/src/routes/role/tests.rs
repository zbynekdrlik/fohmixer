use fohmixer_proto::client::HubStatus;
use fohmixer_proto::roles::Role;

use super::*;

#[test]
fn every_api_route_has_a_role_decision() {
    for path in API_PATHS {
        assert!(route_role(path).is_some(), "{path}");
    }
    assert_eq!(route_role("/api/nothing"), None);
    assert_eq!(route_role("/api/version"), Some(RouteAccess::Open));
    assert_eq!(route_role("/api/auth"), Some(RouteAccess::Open));
    assert_eq!(route_role("/api/client-error"), Some(RouteAccess::Open));
    assert_eq!(route_role("/api/client-report"), Some(RouteAccess::Open));
    assert_eq!(route_role("/api/layout"), Some(RouteAccess::AnyRole));
    assert_eq!(route_role("/api/status"), Some(RouteAccess::AnyRole));
    assert_eq!(route_role("/ws"), Some(RouteAccess::AnyRole));
    for path in ["/api/markers/migration", "/api/eq/picture", "/api/songmap"] {
        assert_eq!(route_role(path), Some(RouteAccess::EngineerOnly), "{path}");
    }
}

#[test]
fn only_an_mbc_token_on_an_engineer_route_or_a_route_without_a_decision_is_refused() {
    for role in [None, Some(Role::Engineer), Some(Role::Mbc)] {
        assert!(!refused(Some(RouteAccess::Open), role), "{role:?}");
        assert!(!refused(Some(RouteAccess::AnyRole), role), "{role:?}");
        assert!(refused(None, role), "{role:?}");
    }
    assert!(refused(Some(RouteAccess::EngineerOnly), Some(Role::Mbc)));
    assert!(!refused(
        Some(RouteAccess::EngineerOnly),
        Some(Role::Engineer)
    ));
    // No token: the handler's own 401.
    assert!(!refused(Some(RouteAccess::EngineerOnly), None));
}

#[test]
fn the_mbc_status_holds_its_live_set_only() {
    let mut status: HubStatus = serde_json::from_value(serde_json::json!({
        "instances": [], "layout": {"rev": 7, "error": "e", "unresolved": [
            {"instance": "band", "target": "t", "error": "x"},
            {"instance": "mbc", "target": "u", "error": "y"}]},
        "stage_aut": {"on": true, "writes": 3}, "clients": 4,
    }))
    .unwrap();
    for name in ["band", "mbc", "master"] {
        status.instances.push(
            serde_json::from_value(serde_json::json!({
                "name": name, "port": 1, "online": true, "busy": false,
                "set_name": "s", "live_version": "v", "script_version": "w",
                "main_tick_age_ms": null, "subscriptions": 2, "listeners": 2,
                "connect_failures": 0, "last_error": null}))
            .unwrap(),
        );
    }
    assert_eq!(status_for(Role::Engineer, status.clone()), status);
    let mbc = status_for(Role::Mbc, status);
    assert_eq!(mbc.instances.len(), 1);
    assert_eq!(mbc.instances[0].name, "mbc");
    assert_eq!(mbc.layout.rev, 7);
    assert_eq!(mbc.layout.error, None);
    assert_eq!(mbc.layout.unresolved.len(), 1);
    assert_eq!(mbc.layout.unresolved[0].target, "u");
    assert!(!mbc.stage_aut.on);
    assert_eq!(mbc.stage_aut.writes, 0);
    assert_eq!(mbc.clients, 0);
}

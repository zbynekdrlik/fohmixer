#!/usr/bin/env python3
"""The Cloudflare side of fohmixer's remote access, as code (#17).

One public name for the LAN and the internet: on the church network the
router's static DNS sends the name to the Ableton PC; everywhere else the
public DNS sends it to a Cloudflare Tunnel whose connector (cloudflared, the
``fohmixer-tunnel`` service the installer sets up) forwards to the hub's plain
HTTP port on the PC, behind a Cloudflare Access application (e-mail one-time
PIN). ``--ableset-name`` adds AbleSet's own name (#90), which the hub serves
beside fohmixer's: the same tunnel and origin, an Access application of its
own (its own AUD, the same allow-list and settings). This script reconciles,
idempotently and in this order:

1. the tunnel (remotely managed): found by name or created;
2. the Access application of each name with its one Allow policy (the
   e-mails given; an empty list is refused: an Access app without an
   allow-list is an open door) - BEFORE the name is published;
3. the tunnel's ingress: every name -> ``--service`` (default the hub's
   ``http://127.0.0.1:8480``, which dispatches by Host), everything else 404.
   The ingress is written whole, so every rule the tunnel has now that the
   wanted list lacks (another host, a path rule, a changed service or
   option, another catch-all) is listed as ``REMOVE route <rule>``, and
   ``--apply`` refuses it before any change unless ``--prune-routes`` is
   given. The rest of the tunnel's configuration is carried over as it is;
4. the public DNS record of each name: a proxied CNAME
   ``<name> -> <tunnel id>.cfargotunnel.com``. A record of the name that is
   not a tunnel CNAME is never overwritten (``--replace-dns`` to allow it).

It then prints the hub's ``[access]`` table (the team domain and the app's AUD
tag, public IDs), with ``--ableset-name`` also its ``[ableset]`` table (the
name and the AbleSet app's AUD), and, with ``--token-out`` and ``--apply``,
writes the tunnel's
connector token to that file (owner-only) for ``Install-Fohmixer.ps1
-SetTunnelToken``; the token is never printed.

DRY-RUN BY DEFAULT: without ``--apply`` it only reads (GET) and says what it
would do. The API token (``--api-token-file``) needs, on the account and the
zone: Account > Cloudflare Tunnel > Edit, Account > Access: Apps and Policies
> Edit, Zone > DNS > Edit (and Account > Access: Organizations > Read for the
team domain). It is read from the file and never printed. Stdlib only, the
pattern of airuleset's ``cli_webterm_access.py``. No site value lives in this
repository (spec 5.2): account, zone, name and e-mails are arguments.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request

API = "https://api.cloudflare.com/client/v4"
POLICY_NAME = "fohmixer allowed users"
TUNNEL_SUFFIX = ".cfargotunnel.com"


class Api:
    """A thin Cloudflare API client. ``transport(method, path, body)`` returns
    ``(status, parsed_json)``; the default calls the real API with the bearer
    token; the tests inject a fake one. ``calls`` records every call."""

    def __init__(self, token=None, transport=None):
        self._token = token
        self._transport = transport or self._http
        self.calls = []

    def _http(self, method, path, body):
        data = json.dumps(body).encode("utf-8") if body is not None else None
        request = urllib.request.Request(
            API + path,
            data=data,
            method=method,
            headers={
                "Authorization": "Bearer " + (self._token or ""),
                "Content-Type": "application/json",
            },
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                return response.status, json.load(response)
        except urllib.error.HTTPError as e:
            try:
                return e.code, json.load(e)
            except ValueError:
                return e.code, {"success": False, "errors": [{"message": "HTTP %s" % e.code}]}

    def call(self, method, path, body=None):
        """One call: its ``result``; a failure raises ``ApiError``."""
        self.calls.append((method, path))
        status, answer = self._transport(method, path, body)
        if status in (200, 201) and answer.get("success"):
            return answer.get("result")
        errors = answer.get("errors") or [{}]
        raise ApiError(
            "%s %s: HTTP %s: %s" % (method, path, status, errors[0].get("message", answer))
        )


class ApiError(RuntimeError):
    """A Cloudflare API call that failed."""


def norm(name):
    """A host name for comparison: case-folded, without a trailing dot or slash."""
    return (name or "").strip().rstrip("/").rstrip(".").casefold()


def access_payload(name, emails, session_duration):
    """The self-hosted Access application of ``name`` with its Allow policy
    inline (one call creates or updates both: no deny-all window)."""
    return {
        "name": "fohmixer - " + name,
        "domain": name,
        "type": "self_hosted",
        "session_duration": session_duration,
        "auto_redirect_to_identity": False,
        "app_launcher_visible": False,
        "policies": [
            {
                "name": POLICY_NAME,
                "decision": "allow",
                "precedence": 1,
                "include": [{"email": {"email": e}} for e in emails],
            }
        ],
    }


def ingress(names, service):
    """The tunnel's configuration: every name to the hub, everything else 404."""
    routes = [{"hostname": n, "service": service, "originRequest": {}} for n in names]
    return {"config": {"ingress": routes + [{"service": "http_status:404"}]}}


def rule_settings(rule, fold=False):
    """An ingress rule as compared: every key with a value (an empty ``path``
    or ``originRequest`` is none); with ``fold``, the host name normalised."""
    settings = {k: v for k, v in (rule or {}).items() if v not in (None, "", {}, [])}
    if fold and "hostname" in settings:
        settings["hostname"] = norm(settings["hostname"])
    return settings


def same_ingress(current, wanted):
    """Whether the tunnel already routes exactly like ``wanted``: every rule
    with all its settings, in order (cloudflared takes the first match)."""
    have = [rule_settings(r) for r in (current or {}).get("ingress") or []]
    want = [rule_settings(r) for r in wanted["config"]["ingress"]]
    return have == want


def lost_rules(current, wanted):
    """The rules the tunnel has now that ``wanted`` lacks: another host, a
    path rule, a changed service or option, another catch-all. The ingress is
    written whole, so the write would remove or change each of them."""
    keep = [rule_settings(r, fold=True) for r in wanted["config"]["ingress"]]
    return [
        r for r in (current or {}).get("ingress") or [] if rule_settings(r, fold=True) not in keep
    ]


def describe_rule(rule):
    """A rule as a REMOVE line names it: ``<host or *>[ path <path>] ->
    <service>``, then any other setting as JSON."""
    rest = rule_settings(rule)
    text = rest.pop("hostname", "*")
    if "path" in rest:
        text += " path %s" % rest.pop("path")
    text += " -> %s" % rest.pop("service", None)
    if rest:
        text += " " + json.dumps(rest, sort_keys=True)
    return text


class Plan:
    """What a run did (or would do) and what the hub needs."""

    def __init__(self, apply):
        self.apply = apply
        self.actions = []
        self.tunnel_id = None
        self.aud = None
        self.team_domain = None
        self.ableset_name = None
        self.ableset_aud = None

    def note(self, text):
        self.actions.append(text)


def reconcile_tunnel(api, plan, account, tunnel_name):
    base = "/accounts/%s/cfd_tunnel" % account
    found = api.call("GET", "%s?name=%s&is_deleted=false" % (base, tunnel_name)) or []
    if found:
        plan.tunnel_id = found[0]["id"]
        plan.note("tunnel %s exists (id %s)" % (tunnel_name, plan.tunnel_id))
        return
    plan.note("create tunnel %s (remotely managed)" % tunnel_name)
    if plan.apply:
        plan.tunnel_id = api.call("POST", base, {"name": tunnel_name, "config_src": "cloudflare"})[
            "id"
        ]


def reconcile_access(api, plan, account, name, emails, session_duration):
    """The Access app of ``name``; its AUD tag (None before it exists)."""
    if not emails:
        raise ApiError(
            "no e-mail for the Access policy: an Access app without an allow-list is an open door"
        )
    base = "/accounts/%s/access" % account
    apps = api.call("GET", base + "/apps?per_page=1000") or []
    app = next((a for a in apps if norm(a.get("domain")) == norm(name)), None)
    payload = access_payload(name, emails, session_duration)
    if app is None:
        plan.note("create the Access app of %s (allow: %d e-mail(s))" % (name, len(emails)))
        if plan.apply:
            app = api.call("POST", base + "/apps", payload)
    else:
        plan.note(
            "update the Access app of %s (id %s, allow: %d e-mail(s))"
            % (name, app.get("id"), len(emails))
        )
        if plan.apply:
            app = api.call("PUT", "%s/apps/%s" % (base, app["id"]), payload)
    return (app or {}).get("aud")


def read_team_domain(api, plan, account):
    """The Access team domain (None when the token cannot read it)."""
    try:
        path = "/accounts/%s/access/organizations" % account
        return (api.call("GET", path) or {}).get("auth_domain")
    except ApiError as e:
        plan.note(
            "the team domain is not readable with this token (%s): Zero Trust > Settings shows it"
            % e
        )
        return None


def config_path(account, plan):
    return "/accounts/%s/cfd_tunnel/%s/configurations" % (account, plan.tunnel_id)


def read_routes(api, plan, account, wanted, prune):
    """The tunnel's configuration now (None for a new tunnel), read before
    any change: with --apply, a rule the write of ``wanted`` would remove or
    change (``lost_rules``) is refused here unless ``prune``
    (--prune-routes)."""
    if plan.tunnel_id is None:
        return None
    current = (api.call("GET", config_path(account, plan)) or {}).get("config")
    lost = lost_rules(current, wanted)
    if lost and plan.apply and not prune:
        raise ApiError(
            "the tunnel has %s, which this run would remove or change: name it, "
            "or pass --prune-routes" % "; ".join(describe_rule(r) for r in lost)
        )
    return current


def reconcile_ingress(api, plan, account, wanted, current, prune):
    """The tunnel's ingress as ``wanted``. The rest of its configuration
    (warp-routing, the default originRequest) is not this script's: the
    write carries it over as it is."""
    routes = [r for r in wanted["config"]["ingress"] if r.get("hostname")]
    shown = "%s -> %s" % (", ".join(r["hostname"] for r in routes), routes[0]["service"])
    if plan.tunnel_id is None:
        plan.note("route %s in the new tunnel" % shown)
        return
    for rule in lost_rules(current, wanted):
        hint = "" if prune else " (--apply needs --prune-routes for it)"
        plan.note("REMOVE route %s%s" % (describe_rule(rule), hint))
    if same_ingress(current, wanted):
        plan.note("the tunnel routes %s" % shown)
        return
    plan.note("route %s in the tunnel" % shown)
    if plan.apply:
        kept = {k: v for k, v in (current or {}).items() if k != "ingress" and v is not None}
        config = dict(kept, ingress=wanted["config"]["ingress"])
        api.call("PUT", config_path(account, plan), {"config": config})


def reconcile_dns(api, plan, zone, name, replace):
    zones = api.call("GET", "/zones?name=%s" % zone) or []
    if not zones:
        raise ApiError("no zone %s for this token" % zone)
    base = "/zones/%s/dns_records" % zones[0]["id"]
    target = "%s%s" % (plan.tunnel_id or "<new tunnel>", TUNNEL_SUFFIX)
    record = {
        "type": "CNAME",
        "name": name,
        "content": target,
        "proxied": True,
        "comment": "fohmixer tunnel (#17)",
    }
    existing = api.call("GET", "%s?name=%s" % (base, name)) or []
    ours = [
        r
        for r in existing
        if r.get("type") == "CNAME" and norm(r.get("content")).endswith(TUNNEL_SUFFIX)
    ]
    foreign = [r for r in existing if r not in ours]
    if foreign and not replace:
        raise ApiError(
            "%s already has a %s record (%s): not ours to overwrite; remove it or pass --replace-dns"
            % (name, foreign[0].get("type"), foreign[0].get("content"))
        )
    for r in foreign:
        plan.note("delete the %s record of %s (%s)" % (r.get("type"), name, r.get("content")))
        if plan.apply:
            api.call("DELETE", "%s/%s" % (base, r["id"]))
    if ours and norm(ours[0].get("content")) == norm(target) and ours[0].get("proxied"):
        plan.note("DNS %s -> %s (proxied)" % (name, target))
        return
    if ours:
        plan.note("update DNS %s -> %s (proxied)" % (name, target))
        if plan.apply:
            api.call("PUT", "%s/%s" % (base, ours[0]["id"]), record)
        return
    plan.note("create DNS %s -> %s (proxied)" % (name, target))
    if plan.apply:
        api.call("POST", base, record)


def write_token(api, plan, account, path):
    """The tunnel's connector token into ``path`` (owner-only), never printed."""
    if not plan.apply or plan.tunnel_id is None:
        plan.note("write the connector token to %s" % path)
        return
    token = api.call("GET", "/accounts/%s/cfd_tunnel/%s/token" % (account, plan.tunnel_id))
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    # An existing file keeps its mode through O_CREAT: narrow it before the
    # token is in it.
    os.fchmod(fd, 0o600)
    with os.fdopen(fd, "w", encoding="ascii") as f:
        f.write(token)
    plan.note("wrote the connector token to %s" % path)


def wanted_names(args):
    """fohmixer's name, then AbleSet's when given (#90)."""
    return [args.name] + ([args.ableset_name] if args.ableset_name else [])


def reconcile(api, args):
    """The whole run, in its order (see the module documentation)."""
    plan = Plan(args.apply)
    names = wanted_names(args)
    plan.ableset_name = args.ableset_name
    reconcile_tunnel(api, plan, args.account_id, args.tunnel_name)
    wanted = ingress(names, args.service)
    current = read_routes(api, plan, args.account_id, wanted, args.prune_routes)
    auds = [
        reconcile_access(api, plan, args.account_id, n, args.email, args.session_duration)
        for n in names
    ]
    plan.aud = auds[0]
    if args.ableset_name:
        plan.ableset_aud = auds[1]
    plan.team_domain = read_team_domain(api, plan, args.account_id)
    reconcile_ingress(api, plan, args.account_id, wanted, current, args.prune_routes)
    for n in names:
        reconcile_dns(api, plan, args.zone, n, args.replace_dns)
    if args.token_out:
        write_token(api, plan, args.account_id, args.token_out)
    return plan


def hub_access_table(plan):
    """The hub's ``[access]`` table, and its ``[ableset]`` table with
    AbleSet's name (the installer's -AbleSetName and -AbleSetAccessAud)."""
    text = '[access]\nteam_domain = "%s"\naud = ["%s"]\n' % (
        plan.team_domain or "<team>.cloudflareaccess.com",
        plan.aud or "<the Access app's AUD, after --apply>",
    )
    if plan.ableset_name:
        text += '\n[ableset]\nname = "%s"\naccess_aud = ["%s"]\n' % (
            plan.ableset_name,
            plan.ableset_aud or "<the AbleSet Access app's AUD, after --apply>",
        )
    return text


def parse_args(argv):
    p = argparse.ArgumentParser(
        description="fohmixer remote access on Cloudflare (#17); dry-run unless --apply"
    )
    p.add_argument("--account-id", required=True)
    p.add_argument("--zone", required=True, help="the DNS zone of the name")
    p.add_argument("--name", required=True, help="the public name, e.g. foh.example.org")
    p.add_argument(
        "--ableset-name",
        default=None,
        help="AbleSet's own name beside it (#90), e.g. ableset.example.org",
    )
    p.add_argument("--tunnel-name", default="fohmixer")
    p.add_argument(
        "--email", action="append", default=[], help="an e-mail the Access policy allows (repeat)"
    )
    p.add_argument(
        "--service", default="http://127.0.0.1:8480", help="the tunnel's origin on the PC"
    )
    p.add_argument("--session-duration", default="720h")
    p.add_argument("--api-token-file", required=True)
    p.add_argument(
        "--token-out", default=None, help="write the tunnel's connector token here (with --apply)"
    )
    p.add_argument("--replace-dns", action="store_true")
    p.add_argument(
        "--prune-routes",
        action="store_true",
        help="with --apply, remove a tunnel route of a name this run does not name",
    )
    p.add_argument("--apply", action="store_true")
    return p.parse_args(argv)


def main(argv=None, transport=None):
    args = parse_args(argv)
    if args.ableset_name and norm(args.ableset_name) == norm(args.name):
        print(
            "fohmixer-cloudflare: --ableset-name is --name: AbleSet needs a name of its own",
            file=sys.stderr,
        )
        return 1
    try:
        with open(os.path.expanduser(args.api_token_file), encoding="utf-8") as f:
            token = f.read().strip()
    except OSError as e:
        print("fohmixer-cloudflare: cannot read the API token file: %s" % e, file=sys.stderr)
        return 1
    if not token:
        print("fohmixer-cloudflare: the API token file is empty", file=sys.stderr)
        return 1
    api = Api(token=token, transport=transport)
    mode = "APPLY" if args.apply else "DRY-RUN (reads only)"
    print("fohmixer-cloudflare [%s] %s" % (mode, ", ".join(wanted_names(args))))
    try:
        plan = reconcile(api, args)
    except ApiError as e:
        print("fohmixer-cloudflare: FAILED: %s" % e, file=sys.stderr)
        return 1
    for action in plan.actions:
        print("  " + action)
    print("the hub's config (fohmixer-hub.toml):\n" + hub_access_table(plan))
    if not args.apply:
        print("(dry-run: nothing changed; run again with --apply)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

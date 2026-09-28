"""Tests of scripts/cloudflare/fohmixer_cloudflare.py (#17) against a fake
Cloudflare API that keeps state: dry-run reads only, apply creates in the safe
order (Access before the name is published), a second apply changes nothing,
a foreign DNS record is never overwritten, the connector token is written
owner-only and never printed."""

from __future__ import annotations

import contextlib
import io
import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fohmixer_cloudflare as fc  # noqa: E402

ACCOUNT = "acc1"
ZONE = "example.org"
NAME = "foh.example.org"
# A low-entropy stand-in: a realistic token would read as a leak to the secret scan.
CONNECTOR = "c" * 120


class FakeCloudflare:
    """Tunnels, Access apps, tunnel configurations, zones and DNS records in
    memory, answering the paths the script calls."""

    def __init__(self):
        self.tunnels = {}
        self.configs = {}
        self.apps = {}
        self.records = {}
        self.zones = {ZONE: "zone1"}
        self.org = {"auth_domain": "team.cloudflareaccess.com"}
        self.org_readable = True
        self.writes = []
        self.next = 0
        self.fail = None

    def _id(self, kind):
        self.next += 1
        return "%s%d" % (kind, self.next)

    def __call__(self, method, path, body):
        if self.fail and self.fail in path:
            return 403, {"success": False, "errors": [{"message": "Authentication error"}]}
        if method != "GET":
            self.writes.append((method, path))
        ok = lambda result: (200, {"success": True, "errors": [], "result": result})  # noqa: E731
        route, _, query = path.partition("?")
        params = dict(p.split("=", 1) for p in query.split("&") if p)
        parts = route.strip("/").split("/")
        if parts[:3] == ["accounts", ACCOUNT, "cfd_tunnel"]:
            if len(parts) == 3 and method == "GET":
                return ok([t for t in self.tunnels.values() if t["name"] == params["name"]])
            if len(parts) == 3 and method == "POST":
                tid = self._id("tun")
                self.tunnels[tid] = {"id": tid, "name": body["name"], "config_src": body["config_src"]}
                return ok(self.tunnels[tid])
            if parts[4] == "configurations" and method == "GET":
                return ok({"config": self.configs.get(parts[3])})
            if parts[4] == "configurations" and method == "PUT":
                self.configs[parts[3]] = body["config"]
                return ok({"config": body["config"]})
            if parts[4] == "token":
                return ok(CONNECTOR)
        if parts[:3] == ["accounts", ACCOUNT, "access"]:
            if parts[3] == "organizations":
                if not self.org_readable:
                    return 403, {"success": False, "errors": [{"message": "no org read"}]}
                return ok(self.org)
            if len(parts) == 4 and method == "GET":
                return ok(list(self.apps.values()))
            if len(parts) == 4 and method == "POST":
                aid = self._id("app")
                self.apps[aid] = dict(body, id=aid, aud="aud-" + aid)
                return ok(self.apps[aid])
            if method == "PUT":
                self.apps[parts[4]] = dict(body, id=parts[4], aud=self.apps[parts[4]]["aud"])
                return ok(self.apps[parts[4]])
        if parts == ["zones"]:
            zid = self.zones.get(params["name"])
            return ok([{"id": zid, "name": params["name"]}] if zid else [])
        if parts[0] == "zones" and parts[2] == "dns_records":
            if len(parts) == 3 and method == "GET":
                return ok([r for r in self.records.values() if r["name"] == params["name"]])
            if len(parts) == 3 and method == "POST":
                rid = self._id("rec")
                self.records[rid] = dict(body, id=rid)
                return ok(self.records[rid])
            if method == "PUT":
                self.records[parts[3]] = dict(body, id=parts[3])
                return ok(self.records[parts[3]])
            if method == "DELETE":
                del self.records[parts[3]]
                return ok({"id": parts[3]})
        return 404, {"success": False, "errors": [{"message": "no route %s %s" % (method, path)}]}


class ScriptTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="fohmixer-cf-test-")
        self.token_file = os.path.join(self.dir, "api-token")
        Path(self.token_file).write_text("t" * 40 + "\n", encoding="utf-8")
        self.cf = FakeCloudflare()

    def run_main(self, *extra, apply=True, emails=("a@example.org", "b@example.org")):
        argv = ["--account-id", ACCOUNT, "--zone", ZONE, "--name", NAME, "--api-token-file", self.token_file]
        for e in emails:
            argv += ["--email", e]
        if apply:
            argv.append("--apply")
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = fc.main(argv + list(extra), transport=self.cf)
        return code, out.getvalue(), err.getvalue()

    def test_a_dry_run_only_reads(self):
        code, out, _ = self.run_main(apply=False)
        self.assertEqual(code, 0, out)
        self.assertEqual(self.cf.writes, [])
        self.assertIn("DRY-RUN", out)
        self.assertIn("create tunnel fohmixer", out)
        self.assertIn("create the Access app of foh.example.org (allow: 2 e-mail(s))", out)
        self.assertIn("create DNS foh.example.org -> <new tunnel>.cfargotunnel.com (proxied)", out)
        self.assertIn("(dry-run: nothing changed", out)

    def test_apply_sets_it_all_up_access_before_the_name_is_published(self):
        token_out = os.path.join(self.dir, "tunnel-token")
        code, out, err = self.run_main("--token-out", token_out)
        self.assertEqual(code, 0, err)
        (tid,) = self.cf.tunnels
        self.assertEqual(self.cf.tunnels[tid]["config_src"], "cloudflare")
        (app,) = self.cf.apps.values()
        self.assertEqual(app["domain"], NAME)
        self.assertEqual(app["type"], "self_hosted")
        self.assertEqual(app["session_duration"], "720h")
        self.assertEqual(
            app["policies"][0]["include"],
            [{"email": {"email": "a@example.org"}}, {"email": {"email": "b@example.org"}}],
        )
        self.assertEqual(app["policies"][0]["decision"], "allow")
        self.assertEqual(
            self.cf.configs[tid]["ingress"],
            [
                {"hostname": NAME, "service": "http://127.0.0.1:8480", "originRequest": {}},
                {"service": "http_status:404"},
            ],
        )
        (record,) = self.cf.records.values()
        self.assertEqual(
            (record["type"], record["name"], record["content"], record["proxied"]),
            ("CNAME", NAME, tid + ".cfargotunnel.com", True),
        )
        kinds = [path.split("/")[3] if path.startswith("/accounts") else "dns" for _, path in self.cf.writes]
        self.assertLess(kinds.index("access"), kinds.index("dns"), "Access exists before the name is public")
        self.assertIn('[access]\nteam_domain = "team.cloudflareaccess.com"\naud = ["aud-' + app["id"] + '"]', out)
        # The connector token: in the file, owner-only, never printed.
        self.assertEqual(Path(token_out).read_text(encoding="ascii"), CONNECTOR)
        self.assertEqual(stat.S_IMODE(os.stat(token_out).st_mode), 0o600)
        self.assertNotIn(CONNECTOR, out + err)
        # A second run changes nothing (the token is fetched again, a read).
        self.cf.writes.clear()
        code, out, _ = self.run_main()
        self.assertEqual(code, 0)
        self.assertEqual(self.cf.writes, [("PUT", "/accounts/acc1/access/apps/" + app["id"])])
        self.assertIn("tunnel fohmixer exists", out)
        self.assertIn("the tunnel routes foh.example.org -> http://127.0.0.1:8480", out)
        self.assertIn("DNS foh.example.org -> %s.cfargotunnel.com (proxied)" % tid, out)

    def test_changes_are_corrected(self):
        self.run_main()
        (tid,) = self.cf.tunnels
        (rid,) = self.cf.records
        self.cf.records[rid]["content"] = "old.cfargotunnel.com"
        self.cf.configs[tid]["ingress"][0]["service"] = "http://127.0.0.1:9999"
        code, out, _ = self.run_main("--service", "http://127.0.0.1:8480", emails=["c@example.org"])
        self.assertEqual(code, 0)
        self.assertEqual(self.cf.records[rid]["content"], tid + ".cfargotunnel.com")
        self.assertEqual(self.cf.configs[tid]["ingress"][0]["service"], "http://127.0.0.1:8480")
        (app,) = self.cf.apps.values()
        self.assertEqual(app["policies"][0]["include"], [{"email": {"email": "c@example.org"}}])
        self.assertIn("update DNS", out)
        self.assertIn("route foh.example.org -> http://127.0.0.1:8480 in the tunnel", out)

    def test_a_foreign_dns_record_is_never_overwritten(self):
        self.cf.records["a1"] = {"id": "a1", "type": "A", "name": NAME, "content": "203.0.113.9"}
        code, _, err = self.run_main()
        self.assertEqual(code, 1)
        self.assertIn("already has a A record (203.0.113.9): not ours to overwrite", err)
        self.assertIn("a1", self.cf.records)
        code, out, err = self.run_main("--replace-dns")
        self.assertEqual(code, 0, err)
        self.assertNotIn("a1", self.cf.records)
        self.assertIn("delete the A record of foh.example.org (203.0.113.9)", out)

    def test_an_empty_allow_list_is_refused_before_anything_is_published(self):
        code, _, err = self.run_main(emails=[])
        self.assertEqual(code, 1)
        self.assertIn("an Access app without an allow-list is an open door", err)
        self.assertEqual(self.cf.records, {})
        self.assertEqual(self.cf.apps, {})

    def test_api_errors_and_token_files(self):
        self.cf.fail = "/access/apps"
        code, _, err = self.run_main()
        self.assertEqual(code, 1)
        self.assertIn("FAILED: GET /accounts/acc1/access/apps?per_page=1000: HTTP 403: Authentication error", err)
        self.assertEqual(self.cf.records, {}, "nothing published after a failed Access step")
        self.cf.fail = None
        self.cf.zones = {}
        code, _, err = self.run_main()
        self.assertIn("no zone example.org for this token", err)
        Path(self.token_file).write_text("\n", encoding="utf-8")
        self.assertEqual(self.run_main()[0], 1)
        os.remove(self.token_file)
        code, _, err = self.run_main()
        self.assertEqual(code, 1)
        self.assertIn("cannot read the API token file", err)

    def test_an_unreadable_team_domain_is_a_note_not_a_failure(self):
        self.cf.org_readable = False
        code, out, _ = self.run_main()
        self.assertEqual(code, 0)
        self.assertIn("the team domain is not readable with this token", out)
        self.assertIn('team_domain = "<team>.cloudflareaccess.com"', out)

    def test_the_real_transport_reports_http_errors(self):
        api = fc.Api(token="t", transport=lambda m, p, b: (500, {"success": False}))
        with self.assertRaises(fc.ApiError) as raised:
            api.call("GET", "/zones")
        self.assertIn("GET /zones: HTTP 500", str(raised.exception))
        self.assertEqual(fc.norm("FOH.Example.org./"), "foh.example.org")
        self.assertTrue(fc.same_ingress(fc.ingress(NAME, "s")["config"], fc.ingress(NAME, "s")))
        self.assertFalse(fc.same_ingress(None, fc.ingress(NAME, "s")))


if __name__ == "__main__":
    unittest.main()

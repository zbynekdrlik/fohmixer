"""The fake iemmixer page (#166, ``fake_iem.py``) as a browser sees it: over
TLS on a name of its own, its ``frame-ancestors``, its timer, the loads
counted, and the tool in a process of its own."""

import http.client
import os
import shutil
import ssl
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import fake_iem  # noqa: E402

ANCESTORS = "https://foh.e2e.test:8443"


def leaf(folder):
    """A self-signed certificate and key for a test: their paths."""
    cert = os.path.join(folder, "cert.pem")
    key = os.path.join(folder, "key.pem")
    subprocess.run(
        [
            *("openssl", "req", "-x509", "-newkey", "ec"),
            *("-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes"),
            *("-keyout", key, "-out", cert, "-days", "1"),
            *("-subj", "/CN=iem.e2e.test", "-addext", "subjectAltName=DNS:iem.e2e.test"),
        ],
        check=True,
        capture_output=True,
    )
    return cert, key


def get(port, path):
    """One GET over TLS, trusting anything: (status, headers, body)."""
    context = ssl.create_default_context()
    context.check_hostname = False
    context.verify_mode = ssl.CERT_NONE
    conn = http.client.HTTPSConnection("127.0.0.1", port, timeout=5, context=context)
    try:
        conn.request("GET", path)
        answer = conn.getresponse()
        return answer.status, dict(answer.getheaders()), answer.read()
    finally:
        conn.close()


class Certificates(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.folder = tempfile.mkdtemp(prefix="fohmixer-fake-iem-test-")
        cls.cert, cls.key = leaf(cls.folder)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.folder, ignore_errors=True)


class Page(Certificates):
    def setUp(self):
        self.fake = fake_iem.FakeIem(0, self.cert, self.key, ANCESTORS)
        self.fake.start()
        self.addCleanup(self.fake.stop)

    def test_the_page_comes_over_tls_and_names_who_may_frame_it(self):
        status, headers, body = get(self.fake.port, "/")
        self.assertEqual(status, 200)
        self.assertEqual(headers["Content-Security-Policy"], f"frame-ancestors {ANCESTORS}")
        self.assertIn(b'id="fake-iem"', body)
        self.assertIn(b"setInterval", body)

    def test_each_load_of_the_page_counts_and_nothing_else_does(self):
        get(self.fake.port, "/")
        get(self.fake.port, "/")
        status, _, _ = get(self.fake.port, "/other")
        self.assertEqual(status, 404)
        state = self.fake.state()
        self.assertEqual((state["port"], state["loads"]), (self.fake.port, 2))
        self.assertEqual(state["hits"], ["/", "/", "/other"])
        self.assertEqual(self.fake.clear()["hits"], [])
        self.assertEqual(self.fake.state()["loads"], 0)

    def test_a_line_it_does_not_know_is_an_error(self):
        self.assertIn("unknown line", fake_iem.command(self.fake, "nothing\n")["error"])
        self.assertEqual(fake_iem.command(self.fake, " state \n")["port"], self.fake.port)


class Refused(Certificates):
    def test_a_key_it_cannot_read_is_a_refused_start(self):
        fake = fake_iem.FakeIem(0, self.cert, os.path.join(self.folder, "missing"))
        with self.assertRaisesRegex(RuntimeError, "cannot listen"):
            fake.start()


class ChildTest(Certificates):
    def test_the_tool_in_a_process_of_its_own(self):
        work = fake_iem.Child(0, self.cert, self.key, ANCESTORS)
        work.start()
        self.addCleanup(work.stop)
        self.assertEqual(get(work.port, "/")[0], 200)
        state = work.state()
        self.assertEqual((state["port"], state["loads"]), (work.port, 1))
        self.assertEqual(work.clear()["loads"], 0)
        work.stop()
        self.assertEqual(work.proc.poll(), 0)


if __name__ == "__main__":
    unittest.main()

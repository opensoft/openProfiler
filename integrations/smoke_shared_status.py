"""Exercise the real service and deployed Python client over local fixture HTTPS.

Build the workspace service with Cargo and set the disposable database engine and URI
in OPENPROFILER_TEST_DB_ENGINE and OPENPROFILER_TEST_DB_URL.
The fixture generates short-lived synthetic signing keys and uses loopback only.
"""

import base64
import http.client
import http.server
import json
import os
from pathlib import Path
import ssl
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
from uuid import uuid4

from openprofiler_status import AccountStatusClient, AccountStatusError


def encoded(value):
    return base64.urlsafe_b64encode(value).rstrip(b"=").decode()


def serve(handler, context=None):
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    if context:
        server.socket = context.wrap_socket(server.socket, server_side=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


class Quiet(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass


def main():
    worktree = Path(__file__).resolve().parents[1]
    binary = worktree / "target/debug/opensoft-open-profiler-account-status"
    engine = os.environ["OPENPROFILER_TEST_DB_ENGINE"]
    database = os.environ["OPENPROFILER_TEST_DB_URL"]
    with tempfile.TemporaryDirectory() as work:
        root = Path(work)
        key, certificate = root / "key.pem", root / "cert.pem"
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                        "-days", "1", "-subj", "/CN=127.0.0.1",
                        "-addext", "subjectAltName=IP:127.0.0.1",
                        "-keyout", str(key), "-out", str(certificate)], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        modulus = subprocess.check_output(["openssl", "rsa", "-in", str(key), "-noout", "-modulus"])
        jwks = json.dumps({"keys": [{"kty": "RSA", "kid": "fixture", "alg": "RS256",
                                   "n": encoded(bytes.fromhex(modulus.decode().strip().split("=")[1])),
                                   "e": "AQAB"}]}).encode()

        class Issuer(Quiet):
            def do_GET(self):
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(jwks)

        issuer_server = serve(Issuer)
        issuer = f"http://127.0.0.1:{issuer_server.server_port}"
        company = f"fixture-{uuid4().hex}"
        actor = lambda identity: {"subject": identity, "client_id": "fixture"}
        settings = root / "identity.json"
        settings.write_text(json.dumps({"issuer": issuer, "audience": "account-status",
            "jwks_url": issuer + "/jwks", "publishers": {company: [actor("publisher")]}}))
        dsn = root / "database-uri"
        dsn.write_text(database)
        account_route = f"/v1/companies/{company}/accounts/claude/account-a"
        routes = (
            ("GET", account_route),
            ("POST", account_route + "/admissions"),
            ("PUT", account_route + "/status"),
            ("PUT", f"/v1/control/companies/{company}/binding-projection"),
        )
        backend_port = 48374
        backend = f"http://127.0.0.1:{backend_port}"
        process = subprocess.Popen([binary], env={
            **os.environ, "STATUS_LOCAL_FIXTURE": "1", "STATUS_LISTEN": f"127.0.0.1:{backend_port}",
            "STATUS_DB_ENGINE": engine, "STATUS_IDENTITY_FILE": str(settings), "STATUS_DB_URL_FILE": str(dsn),
        })
        try:
            for _ in range(50):
                assert process.poll() is None, "Service exited before readiness"
                try:
                    with urllib.request.urlopen(backend + "/readyz", timeout=1) as ready:
                        assert ready.status == 204
                        break
                except OSError:
                    time.sleep(0.2)
            else:
                raise RuntimeError("Service did not become ready")

            class Proxy(Quiet):
                def relay(self):
                    route = next(
                        ((method, path) for method, path in routes
                         if self.command == method and self.path == path), None
                    )
                    if route is None:
                        self.send_error(404)
                        return
                    method, path = route
                    try:
                        length = int(self.headers.get("Content-Length", "0"))
                    except ValueError:
                        self.send_error(400)
                        return
                    if not 0 <= length <= 16_384:
                        self.send_error(413)
                        return
                    content = self.rfile.read(length) if length else None
                    headers = {name: self.headers[name] for name in ("Authorization", "Content-Type")
                               if name in self.headers}
                    with_backend = http.client.HTTPConnection("127.0.0.1", backend_port, timeout=5)
                    try:
                        with_backend.request(method, path, content, headers)
                        response = with_backend.getresponse()
                        self.send_response(response.status)
                        self.send_header("Content-Type", "application/json")
                        self.end_headers()
                        self.wfile.write(response.read())
                    finally:
                        with_backend.close()

                do_GET = do_POST = do_PUT = relay

            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.load_cert_chain(certificate, key)
            os.environ["SSL_CERT_FILE"] = str(certificate)
            https = serve(Proxy, context)
            origin = f"https://127.0.0.1:{https.server_port}"

            def token(identity):
                parts = [encoded(json.dumps(part).encode()) for part in (
                    {"kid": "fixture", "alg": "RS256"},
                    {"iss": issuer, "aud": "account-status", "sub": identity,
                     "azp": "fixture", "exp": int(time.time()) + 300},
                )]
                content = ".".join(parts)
                signature = subprocess.check_output(
                    ["openssl", "dgst", "-sha256", "-sign", str(key)], input=content.encode())
                return content + "." + encoded(signature)

            def edit(path, identity, data):
                request = urllib.request.Request(origin + path, json.dumps(data).encode(),
                    {"Authorization": "Bearer " + token(identity), "Content-Type": "application/json"}, method="PUT")
                trust = ssl.create_default_context(cafile=str(certificate))
                with urllib.request.urlopen(request, timeout=5, context=trust) as response:
                    body = response.read()
                    return json.loads(body) if body else None

            account = {"provider": "claude", "account_ref": "account-a"}
            bindings = [("operator", None), ("desktop-two", None), ("hermes", "hermes"),
                        ("medx", "medx"), ("ledger", "ledger")]
            edit(f"/v1/control/companies/{company}/binding-projection", "publisher", {
                "generation": 1, "expires_at": int(time.time()) + 600,
                "members": [actor(name) for name, _ in bindings],
                "accounts": [{"account": account, "display_name": "Fixture"}],
                "grants": [{"principal": actor(name), "account": account, "use_account": True,
                    "manage": name == "operator",
                    "workload": {"environment": "qa", "factory_instance": factory, "role": "runner"} if factory else None}
                    for name, factory in bindings if name != "ledger"],
            })
            route = f"/v1/companies/{company}/accounts/claude/account-a/status"
            revision = None
            for desired in ("active", "paused", "active"):
                result = edit(route, "operator", {"desired_status": desired, "expected_revision": revision,
                                                  "request_id": str(uuid4())})
                revision = result["account"]["revision"]
                for name, operation in (("desktop-two", "activation"), ("hermes", "turn"), ("medx", "job")):
                    token_file = root / name
                    token_file.write_text(token(name))
                    client = AccountStatusClient.from_binding(
                        {"origin": origin, "company_id": company, **account, "token_file": str(token_file)})
                    assert (client.read().status, client.read().revision) == (desired, revision)
                    try:
                        assert client.admit(operation).revision == revision
                    except AccountStatusError as exc:
                        assert desired == "paused" and exc.code == "account_paused"
                    else:
                        assert desired == "active"
            rejected = root / "ledger"
            rejected.write_text(token("ledger"))
            client = AccountStatusClient.from_binding(
                {"origin": origin, "company_id": company, **account, "token_file": str(rejected)})
            try:
                client.read()
            except AccountStatusError as exc:
                assert exc.code == "account_assignment_denied"
            else:
                raise AssertionError("Unassigned Ledger workload could read the account")
            print("HTTPS smoke passed: shared Active/Paused/Active and revision checks, plus unassigned factory denial.")
        finally:
            process.terminate()
            process.wait(timeout=10)


if __name__ == "__main__":
    main()

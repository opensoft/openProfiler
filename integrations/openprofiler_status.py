"""Client for the openProfiler account-status v1 contract."""

from dataclasses import dataclass
import json
import re
import time
from pathlib import Path
from urllib import error, request
from urllib.parse import urlsplit
from uuid import uuid4


class AccountStatusError(ValueError):
    def __init__(self, code):
        self.code = code
        super().__init__(code)


class _NoRedirect(request.HTTPRedirectHandler):
    def redirect_request(self, *_args, **_kwargs):
        return None


def _key(value):
    return (isinstance(value, str) and value not in {".", ".."}
            and re.fullmatch(r"[A-Za-z0-9_.-]{1,128}", value) is not None)


def read_settings(path):
    try:
        with Path(path).open("rb") as stream:
            raw = stream.read(64 * 1024 + 1)
        if len(raw) > 64 * 1024:
            raise ValueError
        value = json.loads(raw)
        if not isinstance(value, dict):
            raise ValueError
        return value
    except (OSError, ValueError, TypeError):
        raise AccountStatusError("account_binding_unavailable") from None


@dataclass(frozen=True)
class AccountStatus:
    account_ref: str
    status: str
    revision: str | None


@dataclass(frozen=True)
class AccountStatusClient:
    origin: str
    company_id: str
    provider: str
    account_ref: str
    token_file: str

    @classmethod
    def from_binding(cls, binding):
        try:
            if set(binding) != {"origin", "company_id", "provider", "account_ref", "token_file"}:
                raise ValueError
            url = urlsplit(binding["origin"])
            if (url.scheme != "https" or not url.hostname or url.username or url.password
                    or url.path not in {"", "/"} or url.query or url.fragment):
                raise ValueError
            if not all(_key(binding[field]) for field in ("company_id", "provider", "account_ref")):
                raise ValueError
            if len(binding["provider"]) > 32 or not Path(binding["token_file"]).is_absolute():
                raise ValueError
            return cls(**binding)
        except (KeyError, ValueError, TypeError):
            raise AccountStatusError("invalid_account_binding") from None

    def _request(self, operation=None):
        url = (
            f"{self.origin.rstrip('/')}/v1/companies/{self.company_id}"
            f"/accounts/{self.provider}/{self.account_ref}"
        )
        try:
            with Path(self.token_file).open("rb") as stream:
                token = stream.read(16 * 1024 + 1).strip()
            if len(token) > 16 * 1024 or re.fullmatch(rb"[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+", token) is None:
                raise AccountStatusError("account_authentication_required")
            headers = {"Authorization": f"Bearer {token.decode('ascii')}", "Accept": "application/json"}
            content = None
            if operation is not None:
                url += "/admissions"
                headers["Content-Type"] = "application/json"
                content = json.dumps(operation).encode()
            issued = time.monotonic()
            with request.build_opener(_NoRedirect).open(request.Request(url, content, headers), timeout=4) as response:
                payload = response.read(16 * 1024 + 1)
            if len(payload) > 16 * 1024 or time.monotonic() - issued > 4:
                raise AccountStatusError("account_status_unavailable")
            return json.loads(payload)
        except error.HTTPError as exc:
            code = "account_assignment_denied" if exc.code in {401, 403, 404} else "account_status_unavailable"
            raise AccountStatusError(code) from None
        except (OSError, ValueError) as exc:
            if isinstance(exc, AccountStatusError):
                raise
            raise AccountStatusError("account_status_unavailable") from None

    def _parse_status(self, result):
        if not isinstance(result, dict) or any(result.get(field) != getattr(self, field)
            for field in ("company_id", "provider", "account_ref")):
            raise AccountStatusError("invalid_account_status_response")
        status = result.get("status")
        revision = result.get("revision")
        if status == "unknown" and revision is None:
            return AccountStatus(self.account_ref, status, revision)
        if (status not in ("paused", "active") or not isinstance(revision, str)
                or not re.fullmatch(r"[1-9][0-9]{0,18}", revision)
                or int(revision) > 2**63 - 1):
            raise AccountStatusError("invalid_account_status_response")
        return AccountStatus(self.account_ref, status, revision)

    def read(self):
        """Read assigned account status, including Paused and Unknown."""
        return self._parse_status(self._request())

    def admit(self, operation_kind):
        operation = {"operation_id": str(uuid4()), "operation_kind": operation_kind}
        result = self._request(operation)
        if not isinstance(result, dict) or any(result.get(key) != value for key, value in operation.items()):
            raise AccountStatusError("invalid_account_status_response")
        account = self._parse_status(result.get("account"))
        decision = result.get("decision")
        if (decision, account.status) == ("allow", "active"):
            return account
        if (decision, account.status) in {("paused", "paused"), ("unknown", "unknown")}:
            raise AccountStatusError(f"account_{decision}")
        raise AccountStatusError("invalid_account_status_response")

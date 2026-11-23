# Hosted company account status

Build the service from the repository root with:

```sh
docker build -f crates/open-profiler-account-status/Dockerfile -t openprofiler-account-status .
```

Terminate HTTPS on the deployment's ingress and keep the container listener on the private service network. Configure the service with:

| Setting                | Value                                                                                                                                                                 |
| ---------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `STATUS_LISTEN`        | Listener address and port. The image uses `0.0.0.0:8080`.                                                                                                             |
| `STATUS_DB_ENGINE`     | `postgres`, `mariadb`, or `mysql`. MariaDB uses the MySQL URL scheme.                                                                                                 |
| `STATUS_DB_URL_FILE`   | Mounted file containing the writer connection URI. The service requires certificate and hostname verification. Supply the driver's CA file parameter for private PKI. |
| `STATUS_IDENTITY_FILE` | Mounted JSON containing `issuer`, `audience`, `jwks_url`, and `publishers`, a company-to-principals map. Principals have `subject` and `client_id`.                   |
| `/readyz`              | Returns 204 for a reachable writer after migrations and startup.                                                                                                      |

Publish each company's complete, short-lived binding projection with
`PUT /v1/control/companies/{company_id}/binding-projection` using a configured
publisher's bearer token. The document includes a monotonic positive `generation`,
an `expires_at` Unix timestamp (at most 24 hours ahead), `members` of type
principal, `accounts` of shape `{"account":{"provider":"claude","account_ref":"account-a"},"display_name":"Account A"}`,
and `grants` each containing `principal`, `account`, `use_account`, `manage`,
and `workload`. For a workload, `workload` contains `environment`,
`factory_instance`, `role`, and optional `host` and `persona`. Use a distinct
authenticated principal for each workload scope. For a human supply null
`workload`. Acknowledge grant changes after the service accepts the new revision.
Maintain publisher renewal before expiry and keep provider credentials in the
existing provider runtimes.

For the desktop, set `OPENPROFILER_COMPANY_CONFIG` to a locally provisioned
JSON file. Configure the public OIDC client with PKCE S256 and loopback
`http://127.0.0.1:<ephemeral-port>/callback` redirects. The service audience must
be in the access token. This example uses non-secret placeholders.

```json
{
  "company_id": "example",
  "service_origin": "https://accounts.example.invalid",
  "authorization_endpoint": "https://identity.example.invalid/realms/example/protocol/openid-connect/auth",
  "token_endpoint": "https://identity.example.invalid/realms/example/protocol/openid-connect/token",
  "client_id": "openprofiler-desktop",
  "managed_roots": [{ "provider": "claude", "path": "company" }],
  "profiles": [
    { "provider": "claude", "path": "company/03", "account_ref": "account-a" }
  ]
}
```

For multiple companies, put the same configuration objects in a top-level `companies` array. Each company needs a distinct ID and distinct managed profile roots. Selecting a company clears the previous sign-in and starts a new session.

Provision bindings for every managed alias and keep managed profiles inside a
declared root. For Codex include the verified `codex_account_id` in each
binding. The native backend uses in-memory access tokens and asks users to sign
in again when the session expires. The company view remains usable even when
a particular workstation has no local login for a listed account.

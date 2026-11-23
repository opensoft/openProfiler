import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import type {
  CompanyAccount,
  CompanyAccountEdit,
  CompanyView,
  Profile,
} from "./types";

const labels = { active: "Active", paused: "Paused", unknown: "Unknown" };
const uncertainSave =
  "The save result is unavailable. Retry the same change to check it";

type EditOutcome =
  "updated" | "unchanged" | "stale_revision" | "request_id_conflict";

export function CompanyAccounts({
  profiles,
  desktopActivePaths,
  windowsPaths = false,
}: {
  profiles: Profile[];
  desktopActivePaths: string[];
  windowsPaths?: boolean;
}) {
  const [view, setView] = useState<CompanyView | null>(null);
  const [checked, setChecked] = useState<Date | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState<CompanyAccountEdit | null>(null);
  const request = useRef(0);

  const refresh = useCallback(async () => {
    const current = ++request.current;
    try {
      const data = await invoke<CompanyView>("company_accounts");
      if (current !== request.current) return;
      setView(data);
      setChecked(data.signed_in ? new Date() : null);
    } catch (reason) {
      if (current !== request.current) return;
      const message = String(reason);
      setView((old) =>
        old
          ? {
              ...old,
              accounts: [],
              bindings: [],
              signed_in:
                old.signed_in && !message.includes("Company session expired"),
            }
          : null,
      );
      setChecked(null);
      setError(message);
    }
  }, []);

  useEffect(() => {
    if (
      import.meta.env.DEV &&
      new URLSearchParams(window.location.search).has("fixture")
    )
      return;
    void refresh();
    return () => {
      ++request.current;
    };
  }, [refresh]);

  async function session(action: "company_sign_in" | "company_sign_out") {
    ++request.current;
    setView((old) =>
      old ? { ...old, accounts: [], bindings: [], signed_in: false } : null,
    );
    setChecked(null);
    setError("");
    setPending(null);
    setBusy(true);
    try {
      await invoke(action);
      await refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function selectCompany(companyId: string) {
    ++request.current;
    setView((old) =>
      old
        ? {
            ...old,
            accounts: [],
            bindings: [],
            company_id: companyId,
            signed_in: false,
          }
        : null,
    );
    setChecked(null);
    setError("");
    setPending(null);
    setBusy(true);
    try {
      await invoke("company_select", { companyId });
      await refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function save(edit: CompanyAccountEdit) {
    setBusy(true);
    setError("");
    try {
      const result = await invoke<{ outcome: EditOutcome }>(
        "company_set_status",
        { edit },
      );
      let message = "";
      switch (result?.outcome) {
        case "updated":
        case "unchanged":
          break;
        case "stale_revision":
          message =
            "The account changed. Review its current status before saving again.";
          break;
        case "request_id_conflict":
          message =
            "This request ID was already used for a different change. Review the current status before saving again.";
          break;
        default:
          throw uncertainSave;
      }
      setPending(null);
      await refresh();
      if (message) setError(message);
    } catch (reason) {
      const message = String(reason);
      setPending(
        message.startsWith("The save result is unavailable.") ? edit : null,
      );
      await refresh();
      setError(message);
    } finally {
      setBusy(false);
    }
  }

  function change(
    account: CompanyAccount,
    desired_status: "active" | "paused",
  ) {
    void save({
      company_id: account.company_id,
      provider: account.provider,
      account_ref: account.account_ref,
      expected_revision: account.revision,
      request_id: crypto.randomUUID(),
      desired_status,
    });
  }

  if (!view?.company_id && !error) return null;

  const pathKey = (path: string) =>
    windowsPaths ? path.replaceAll("\\", "/").toLowerCase() : path;
  const selectedPaths = desktopActivePaths.map(pathKey);

  return (
    <section className="summary" aria-label="Company accounts">
      <div>
        <h2>
          Company accounts{view?.company_id ? ` · ${view.company_id}` : ""}
        </h2>
        {view && view.companies.length > 1 ? (
          <label>
            Company
            <select
              disabled={busy}
              value={view.company_id ?? ""}
              onChange={(event) => void selectCompany(event.target.value)}
            >
              {view.companies.map((company) => (
                <option key={company}>{company}</option>
              ))}
            </select>
          </label>
        ) : null}
        {view?.signed_in ? (
          <>
            <button
              type="button"
              disabled={busy}
              onClick={() => {
                setError("");
                void refresh();
              }}
            >
              Refresh status
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => void session("company_sign_out")}
            >
              Sign out
            </button>
          </>
        ) : (
          <button
            type="button"
            disabled={busy}
            onClick={() => void session("company_sign_in")}
          >
            {busy ? "Waiting for sign-in" : "Sign in to company"}
          </button>
        )}
        {pending && view?.signed_in ? (
          <button
            type="button"
            disabled={busy}
            onClick={() => void save(pending)}
          >
            Retry save
          </button>
        ) : null}
        {checked ? <p>Checked {checked.toLocaleTimeString()}</p> : null}
        {error ? <p role="alert">{error}</p> : null}
        {view?.signed_in && checked && !view.accounts.length ? (
          <p>No assigned company accounts.</p>
        ) : null}
        {view?.accounts.length ? (
          <ul>
            {view.accounts.map((account) => {
              const paths = view.bindings
                .filter(
                  (b) =>
                    b.provider === account.provider &&
                    b.account_ref === account.account_ref,
                )
                .map((b) => pathKey(b.path));
              const locals = profiles.filter(
                (p) =>
                  p.provider === account.provider &&
                  paths.includes(pathKey(p.profilePath)),
              );
              const selected = locals.some(
                (p) =>
                  p.active ||
                  (p.provider === "codex" &&
                    selectedPaths.includes(pathKey(p.profilePath))),
              );
              return (
                <li key={`${account.provider}/${account.account_ref}`}>
                  <strong>{account.display_name || account.account_ref}</strong>
                  <span>
                    {" "}
                    · {account.provider} · {labels[account.status]}
                    {account.revision ? ` · Revision ${account.revision}` : ""}
                  </span>
                  <span>
                    {locals.some((p) => p.credentialPresent)
                      ? "Local credential present"
                      : "No local credential"}
                    {selected ? " · Selected on this device" : ""}
                  </span>
                  {account.status !== "active" ? (
                    <button
                      type="button"
                      disabled={busy || !!pending}
                      onClick={() => change(account, "active")}
                    >
                      {account.status === "unknown"
                        ? "Set active"
                        : "Reactivate"}
                    </button>
                  ) : null}
                  {account.status !== "paused" ? (
                    <button
                      type="button"
                      disabled={busy || !!pending}
                      onClick={() => change(account, "paused")}
                    >
                      Pause
                    </button>
                  ) : null}
                </li>
              );
            })}
          </ul>
        ) : null}
      </div>
    </section>
  );
}

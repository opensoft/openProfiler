import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useState } from "react";

import {
  ALL_FAMILIES,
  credentialLabel,
  filterProfiles,
  profileFamilies,
} from "./profile-utils";
import type { CodexAction, CodexProfile, ProfileInventory } from "./types";

const actions: Array<{ action: CodexAction; label: string }> = [
  { action: "launch", label: "Launch" },
  { action: "status", label: "Status" },
  { action: "login", label: "Login" },
  { action: "logout", label: "Logout" },
];

function ProfileCard({
  profile,
  onCommand,
}: {
  profile: CodexProfile;
  onCommand: (profile: CodexProfile, action: CodexAction) => Promise<void>;
}) {
  const credentialClass = profile.credentialPresent
    ? "status-chip status-chip--ready"
    : profile.configured
      ? "status-chip status-chip--warning"
      : "status-chip";

  return (
    <article className="profile-card">
      <div className="profile-card__topline">
        <div>
          <p className="eyebrow">{profile.family}</p>
          <h2>{profile.name}</h2>
        </div>
        <span className={credentialClass}>{credentialLabel(profile)}</span>
      </div>

      <dl className="profile-details">
        <div>
          <dt>Expected identity</dt>
          <dd>{profile.email}</dd>
        </div>
        <div>
          <dt>Profile path</dt>
          <dd title={profile.profilePath}>{profile.profilePath}</dd>
        </div>
        <div>
          <dt>Aliases</dt>
          <dd>
            {profile.aliases.length ? profile.aliases.join(", ") : "None"}
          </dd>
        </div>
      </dl>

      <div className="profile-card__footer">
        <span className="source-label">
          {profile.source === "manifest" ? "Manifest" : "Local metadata"}
        </span>
        <div className="actions" aria-label={`Commands for ${profile.name}`}>
          {actions.map(({ action, label }) => (
            <button
              className={
                action === "launch" ? "button button--primary" : "button"
              }
              key={action}
              onClick={() => void onCommand(profile, action)}
              type="button"
            >
              {label}
            </button>
          ))}
        </div>
      </div>
    </article>
  );
}

export default function App() {
  const [inventory, setInventory] = useState<ProfileInventory | null>(null);
  const [family, setFamily] = useState(ALL_FAMILIES);
  const [query, setQuery] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [copied, setCopied] = useState("");
  const [commandPreview, setCommandPreview] = useState("");

  const loadProfiles = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const nextInventory = await invoke<ProfileInventory>(
        "list_codex_profiles",
      );
      setInventory(nextInventory);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadProfiles();
  }, [loadProfiles]);

  const families = useMemo(
    () => profileFamilies(inventory?.profiles ?? []),
    [inventory],
  );
  const visibleProfiles = useMemo(
    () => filterProfiles(inventory?.profiles ?? [], family, query),
    [family, inventory, query],
  );

  const copyCommand = useCallback(
    async (profile: CodexProfile, action: CodexAction) => {
      setError("");
      try {
        const command = await invoke<string>("build_codex_command", {
          profile: profile.name,
          action,
        });
        setCommandPreview(command);
        try {
          await navigator.clipboard.writeText(command);
          setCopied(`${command} copied`);
        } catch {
          setCopied("Command ready to copy");
        }
        window.setTimeout(() => setCopied(""), 2600);
      } catch (reason) {
        setError(String(reason));
      }
    },
    [],
  );

  const readyCount =
    inventory?.profiles.filter((profile) => profile.credentialPresent).length ??
    0;

  return (
    <main className="app-shell">
      <header className="hero">
        <div className="brand-mark" aria-hidden="true">
          <span />
        </div>
        <div className="hero__copy">
          <p className="eyebrow">Opensoft · workBenches</p>
          <h1>Profile Switcher</h1>
          <p>
            Choose the right Codex identity without moving or exposing
            credentials.
          </p>
        </div>
        <div className="privacy-badge">
          <span className="privacy-badge__dot" />
          Local metadata only
        </div>
      </header>

      <section className="summary" aria-label="Profile inventory summary">
        <div>
          <strong>{inventory?.profiles.length ?? 0}</strong>
          <span>Profiles</span>
        </div>
        <div>
          <strong>{readyCount}</strong>
          <span>Ready</span>
        </div>
        <div>
          <strong>{families.length}</strong>
          <span>Families</span>
        </div>
        <button
          className="refresh-button"
          onClick={() => void loadProfiles()}
          type="button"
        >
          <span aria-hidden="true">↻</span>
          Refresh inventory
        </button>
      </section>

      <section className="toolbar" aria-label="Profile filters">
        <label className="search">
          <span className="sr-only">Search profiles</span>
          <span aria-hidden="true">⌕</span>
          <input
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search name, identity, family, or alias"
            type="search"
            value={query}
          />
        </label>
        <div className="family-tabs" role="group" aria-label="Profile family">
          {[ALL_FAMILIES, ...families].map((item) => (
            <button
              aria-pressed={family === item}
              className={
                family === item ? "family-tab family-tab--active" : "family-tab"
              }
              key={item}
              onClick={() => setFamily(item)}
              type="button"
            >
              {item === ALL_FAMILIES ? "All profiles" : item}
            </button>
          ))}
        </div>
      </section>

      {error ? (
        <aside className="message message--error" role="alert">
          <strong>Profile inventory unavailable</strong>
          <span>{error}</span>
        </aside>
      ) : null}

      {copied ? (
        <div className="toast" role="status">
          {copied}
        </div>
      ) : null}

      {commandPreview ? (
        <section
          className="command-preview"
          aria-label="Generated Codex command"
        >
          <div>
            <span>Generated command</span>
            <code>{commandPreview}</code>
          </div>
          <button
            className="button"
            onClick={() => {
              void navigator.clipboard
                .writeText(commandPreview)
                .then(() => setCopied(`${commandPreview} copied`))
                .catch(() =>
                  setCopied("Select the command and copy it manually"),
                );
            }}
            type="button"
          >
            Copy
          </button>
        </section>
      ) : null}

      {loading ? (
        <section className="loading-state" aria-live="polite">
          <span className="spinner" />
          Reading profile metadata…
        </section>
      ) : visibleProfiles.length ? (
        <section className="profile-grid" aria-label="Codex profiles">
          {visibleProfiles.map((profile) => (
            <ProfileCard
              key={profile.name}
              onCommand={copyCommand}
              profile={profile}
            />
          ))}
        </section>
      ) : (
        <section className="empty-state">
          <div className="empty-state__icon" aria-hidden="true">
            &gt;_
          </div>
          <h2>No matching Codex profiles</h2>
          <p>
            Run <code>setup-codex-profiles.sh</code> or adjust the manifest and
            profile home environment variables.
          </p>
        </section>
      )}

      <footer className="inventory-footer">
        <div>
          <span>Manifest</span>
          <code>{inventory?.manifestPath ?? "Waiting for inventory…"}</code>
        </div>
        <div>
          <span>Profile home</span>
          <code>{inventory?.profilesHome ?? "Waiting for inventory…"}</code>
        </div>
      </footer>
    </main>
  );
}

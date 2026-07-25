import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useState } from "react";

import { devFixtureInventory } from "./dev-fixture";
import {
  activationPayload,
  ALL_PROVIDERS,
  credentialLabel,
  filterProfiles,
  providerLabel,
} from "./profile-utils";
import type {
  ActivationResult,
  Profile,
  ProfileInventory,
  Provider,
} from "./types";

function ProfileCard({
  activating,
  onActivate,
  profile,
}: {
  activating: string;
  onActivate: (profile: Profile) => Promise<void>;
  profile: Profile;
}) {
  const key = `${profile.provider}:${profile.profilePath}`;
  const credentialClass = profile.active
    ? "status-chip status-chip--active"
    : profile.credentialPresent
      ? "status-chip status-chip--ready"
      : profile.configured
        ? "status-chip status-chip--warning"
        : "status-chip";

  return (
    <article className={`profile-card profile-card--${profile.provider}`}>
      <div className="profile-card__topline">
        <div>
          <p className="eyebrow">
            {providerLabel(profile.provider)} · {profile.family}
          </p>
          <h2>{profile.name}</h2>
        </div>
        <span className={credentialClass}>{credentialLabel(profile)}</span>
      </div>

      <dl className="profile-details">
        <div>
          <dt>Identity</dt>
          <dd>{profile.email || "Not declared"}</dd>
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
          {profile.source === "manifest"
            ? "Manifest"
            : profile.source === "profile-metadata"
              ? "Local metadata"
              : "Profile directory"}
        </span>
        <button
          className="button button--primary"
          disabled={
            profile.active || !profile.credentialPresent || Boolean(activating)
          }
          onClick={() => void onActivate(profile)}
          type="button"
        >
          {profile.active
            ? "Active"
            : activating === key
              ? "Activating…"
              : "Activate"}
        </button>
      </div>
    </article>
  );
}

export default function App() {
  const [inventory, setInventory] = useState<ProfileInventory | null>(null);
  const [provider, setProvider] = useState<Provider | typeof ALL_PROVIDERS>(
    ALL_PROVIDERS,
  );
  const [query, setQuery] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [toast, setToast] = useState("");
  const [activating, setActivating] = useState("");

  const loadProfiles = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      if (
        import.meta.env.DEV &&
        new URLSearchParams(window.location.search).has("fixture")
      ) {
        setInventory(devFixtureInventory);
        return;
      }
      setInventory(await invoke<ProfileInventory>("list_profiles"));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadProfiles();
  }, [loadProfiles]);

  const showToast = useCallback((message: string) => {
    setToast(message);
    window.setTimeout(() => setToast(""), 3600);
  }, []);

  const visibleProfiles = useMemo(
    () => filterProfiles(inventory?.profiles ?? [], provider, query),
    [inventory, provider, query],
  );

  const activate = useCallback(
    async (profile: Profile) => {
      const key = `${profile.provider}:${profile.profilePath}`;
      setActivating(key);
      setError("");
      try {
        const result = await invoke<ActivationResult>(
          "activate_profile",
          activationPayload(profile),
        );
        await loadProfiles();
        showToast(
          `${providerLabel(result.provider)} profile ${result.profile} is active. Restart the provider app if it was already open.`,
        );
      } catch (reason) {
        setError(String(reason));
      } finally {
        setActivating("");
      }
    },
    [loadProfiles, showToast],
  );

  const readyCount =
    inventory?.profiles.filter((profile) => profile.credentialPresent).length ??
    0;
  const activeCount =
    inventory?.profiles.filter((profile) => profile.active).length ?? 0;
  const issues = inventory?.stores.flatMap((store) => store.issues) ?? [];

  return (
    <main className="app-shell">
      <header className="hero">
        <div className="brand-mark" aria-hidden="true">
          <span />
        </div>
        <div className="hero__copy">
          <p className="eyebrow">Local profiles · Codex + Claude</p>
          <h1>Profile Switcher</h1>
          <p>
            Discover workBenches profiles and choose the local identity each
            provider uses next.
          </p>
        </div>
        <div className="privacy-badge">
          <span className="privacy-badge__dot" />
          Local-only activation
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
          <strong>{activeCount}</strong>
          <span>Active</span>
        </div>
        <button
          className="refresh-button"
          onClick={() => void loadProfiles()}
          type="button"
        >
          <span aria-hidden="true">↻</span>
          Scan profile stores
        </button>
      </section>

      <section className="toolbar" aria-label="Profile filters">
        <label className="search">
          <span className="sr-only">Search profiles</span>
          <span aria-hidden="true">⌕</span>
          <input
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search provider, name, identity, family, or path"
            type="search"
            value={query}
          />
        </label>
        <div className="family-tabs" role="group" aria-label="Provider">
          {[
            { value: ALL_PROVIDERS, label: "All" },
            { value: "codex", label: "Codex" },
            { value: "claude", label: "Claude" },
          ].map((item) => (
            <button
              aria-pressed={provider === item.value}
              className={
                provider === item.value
                  ? "family-tab family-tab--active"
                  : "family-tab"
              }
              key={item.value}
              onClick={() =>
                setProvider(item.value as Provider | typeof ALL_PROVIDERS)
              }
              type="button"
            >
              {item.label}
            </button>
          ))}
        </div>
      </section>

      {error ? (
        <aside className="message message--error" role="alert">
          <strong>Profile operation failed</strong>
          <span>{error}</span>
        </aside>
      ) : null}

      {issues.length ? (
        <aside className="message message--warning" role="status">
          <strong>Some profile metadata could not be loaded</strong>
          {issues.slice(0, 4).map((issue, index) => (
            <span key={`${index}:${issue}`}>{issue}</span>
          ))}
        </aside>
      ) : null}

      {toast ? (
        <div className="toast" role="status">
          {toast}
        </div>
      ) : null}

      {loading ? (
        <section className="loading-state" aria-live="polite">
          <span className="spinner" />
          Scanning Codex and Claude profile stores…
        </section>
      ) : visibleProfiles.length ? (
        <section className="profile-grid" aria-label="Local profiles">
          {visibleProfiles.map((profile) => (
            <ProfileCard
              activating={activating}
              key={`${profile.provider}:${profile.profilePath}`}
              onActivate={activate}
              profile={profile}
            />
          ))}
        </section>
      ) : (
        <section className="empty-state">
          <div className="empty-state__icon" aria-hidden="true">
            &gt;_
          </div>
          <h2>No matching local profiles</h2>
          <p>
            Set up Codex or Claude profiles with workBenches, then scan again.
            The switcher also recognizes credential-bearing profile directories
            when no manifest is available.
          </p>
        </section>
      )}

      <footer className="inventory-footer">
        {(inventory?.stores ?? []).map((store) => (
          <div key={store.provider}>
            <span>{providerLabel(store.provider)} profile store</span>
            <code title={store.profilesHome}>{store.profilesHome}</code>
            <small title={store.activeHome}>
              Active home: {store.activeHome}
            </small>
          </div>
        ))}
      </footer>
    </main>
  );
}

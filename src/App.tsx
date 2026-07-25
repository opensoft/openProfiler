import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useState } from "react";

import { devFixtureDesktopStatus, devFixtureInventory } from "./dev-fixture";
import {
  activationPayload,
  ALL_PROVIDERS,
  credentialLabel,
  desktopActivationPayload,
  desktopActive,
  desktopEligible,
  filterProfiles,
  providerLabel,
} from "./profile-utils";
import type {
  ActivationResult,
  DesktopActivationOutcome,
  DesktopAppStatus,
  Profile,
  ProfileInventory,
  Provider,
} from "./types";

const PROVIDER_FILTERS = [
  { value: ALL_PROVIDERS, label: "All" },
  { value: "codex", label: "Codex" },
  { value: "claude", label: "Claude" },
] as const;

function profileCredentialClass(profile: Profile): string {
  if (profile.active) {
    return "status-chip status-chip--active";
  }
  if (profile.credentialPresent) {
    return "status-chip status-chip--ready";
  }
  if (profile.configured) {
    return "status-chip status-chip--warning";
  }
  return "status-chip";
}

function profileSourceLabel(profile: Profile): string {
  if (profile.source === "manifest") {
    return "Manifest";
  }
  if (profile.source === "profile-metadata") {
    return "Local metadata";
  }
  return "Profile directory";
}

function desktopButtonLabel(
  active: boolean,
  activating: boolean,
  eligible: boolean,
): string {
  if (active) {
    return "GPT app active";
  }
  if (activating) {
    return "Switching GPT app…";
  }
  return eligible ? "Use in GPT app" : "GPT login required";
}

function cliButtonLabel(active: boolean, activating: boolean): string {
  if (active) {
    return "CLI active";
  }
  return activating ? "Activating…" : "Use in CLI";
}

function DesktopProfileAction({
  desktop,
  desktopActivating,
  onActivateDesktop,
  profile,
}: Readonly<{
  desktop: DesktopAppStatus | null;
  desktopActivating: string;
  onActivateDesktop: (profile: Profile) => Promise<void>;
  profile: Profile;
}>) {
  if (profile.provider !== "codex" || !desktop?.platformSupported) {
    return null;
  }

  const key = `${profile.provider}:${profile.profilePath}`;
  const eligible = desktopEligible(profile, desktop);
  const active = desktopActive(profile, desktop);
  const activating = desktopActivating === key;
  const disabled =
    active ||
    !desktop.installed ||
    !desktop.fileActivationSupported ||
    !eligible ||
    desktop.rollbackAvailable ||
    Boolean(desktopActivating);

  return (
    <button
      className="button button--desktop"
      disabled={disabled}
      onClick={() => void onActivateDesktop(profile)}
      title={desktop.message}
      type="button"
    >
      {desktopButtonLabel(active, activating, eligible)}
    </button>
  );
}

function ProfileCard({
  activating,
  desktop,
  desktopActivating,
  onActivate,
  onActivateDesktop,
  profile,
}: Readonly<{
  activating: string;
  desktop: DesktopAppStatus | null;
  desktopActivating: string;
  onActivate: (profile: Profile) => Promise<void>;
  onActivateDesktop: (profile: Profile) => Promise<void>;
  profile: Profile;
}>) {
  const key = `${profile.provider}:${profile.profilePath}`;
  const cliActivating = activating === key;

  return (
    <article className={`profile-card profile-card--${profile.provider}`}>
      <div className="profile-card__topline">
        <div>
          <p className="eyebrow">
            {providerLabel(profile.provider)} · {profile.family}
          </p>
          <h2>{profile.name}</h2>
        </div>
        <span className={profileCredentialClass(profile)}>
          {credentialLabel(profile)}
        </span>
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
        <span className="source-label">{profileSourceLabel(profile)}</span>
        <div className="actions">
          <DesktopProfileAction
            desktop={desktop}
            desktopActivating={desktopActivating}
            onActivateDesktop={onActivateDesktop}
            profile={profile}
          />
          <button
            className="button button--primary"
            disabled={
              profile.active ||
              !profile.credentialPresent ||
              Boolean(activating) ||
              Boolean(desktopActivating)
            }
            onClick={() => void onActivate(profile)}
            type="button"
          >
            {cliButtonLabel(profile.active, cliActivating)}
          </button>
        </div>
      </div>
    </article>
  );
}

function DesktopStatusPanel({
  desktop,
  desktopActivating,
  onConfirm,
  onRollback,
}: Readonly<{
  desktop: DesktopAppStatus | null;
  desktopActivating: string;
  onConfirm: () => Promise<void>;
  onRollback: () => Promise<void>;
}>) {
  if (!desktop?.platformSupported) {
    return null;
  }

  let runtimeStatus = "";
  if (desktop.installed) {
    runtimeStatus = desktop.running ? " · Running" : " · Stopped";
  }

  return (
    <output className="message message--desktop">
      <strong>Windows GPT app</strong>
      <span>
        {desktop.message}
        {runtimeStatus}
      </span>
      {desktop.rollbackAvailable ? (
        <div className="desktop-confirmation">
          <span>
            Check the account and workspace in the GPT app before keeping this
            login.
          </span>
          <div className="actions">
            <button
              className="button button--primary"
              disabled={Boolean(desktopActivating)}
              onClick={() => void onConfirm()}
              type="button"
            >
              {desktopActivating === "__confirm__"
                ? "Keeping…"
                : "Keep this login"}
            </button>
            <button
              className="button"
              disabled={Boolean(desktopActivating)}
              onClick={() => void onRollback()}
              type="button"
            >
              {desktopActivating === "__rollback__"
                ? "Restoring…"
                : "Undo switch"}
            </button>
          </div>
        </div>
      ) : null}
    </output>
  );
}

function ProfileResults({
  activating,
  desktop,
  desktopActivating,
  loading,
  onActivate,
  onActivateDesktop,
  profiles,
}: Readonly<{
  activating: string;
  desktop: DesktopAppStatus | null;
  desktopActivating: string;
  loading: boolean;
  onActivate: (profile: Profile) => Promise<void>;
  onActivateDesktop: (profile: Profile) => Promise<void>;
  profiles: Profile[];
}>) {
  if (loading) {
    return (
      <section className="loading-state" aria-live="polite">
        <span className="spinner" />
        Scanning Codex and Claude profile stores…
      </section>
    );
  }

  if (!profiles.length) {
    return (
      <section className="empty-state">
        <div className="empty-state__icon" aria-hidden="true">
          &gt;_
        </div>
        <h2>No matching local profiles</h2>
        <p>
          Set up Codex or Claude profiles with workBenches, then scan again. The
          switcher also recognizes credential-bearing profile directories when
          no manifest is available.
        </p>
      </section>
    );
  }

  return (
    <section className="profile-grid" aria-label="Local profiles">
      {profiles.map((profile) => (
        <ProfileCard
          activating={activating}
          desktop={desktop}
          desktopActivating={desktopActivating}
          key={`${profile.provider}:${profile.profilePath}`}
          onActivate={onActivate}
          onActivateDesktop={onActivateDesktop}
          profile={profile}
        />
      ))}
    </section>
  );
}

export default function App() {
  const [inventory, setInventory] = useState<ProfileInventory | null>(null);
  const [desktop, setDesktop] = useState<DesktopAppStatus | null>(null);
  const [provider, setProvider] = useState<Provider | typeof ALL_PROVIDERS>(
    ALL_PROVIDERS,
  );
  const [query, setQuery] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [toast, setToast] = useState("");
  const [activating, setActivating] = useState("");
  const [desktopActivating, setDesktopActivating] = useState("");

  const loadProfiles = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      if (
        import.meta.env.DEV &&
        new URLSearchParams(window.location.search).has("fixture")
      ) {
        const fixture =
          new URLSearchParams(window.location.search).get("fixture") ?? "";
        setInventory(devFixtureInventory);
        setDesktop({
          ...devFixtureDesktopStatus,
          rollbackAvailable: fixture === "rollback",
        });
        return;
      }
      setInventory(await invoke<ProfileInventory>("list_profiles"));
      setDesktop(await invoke<DesktopAppStatus>("desktop_app_status"));
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

  const activateDesktop = useCallback(
    async (profile: Profile) => {
      const key = `${profile.provider}:${profile.profilePath}`;
      setDesktopActivating(key);
      setError("");
      try {
        const result = await invoke<DesktopActivationOutcome>(
          "activate_codex_desktop_profile",
          desktopActivationPayload(profile),
        );
        await loadProfiles();
        showToast(
          `${result.profile} is active in the GPT app. Confirm the account in its profile menu, then keep this login or undo it here.`,
        );
      } catch (reason) {
        const message = String(reason);
        await loadProfiles();
        setError(message);
      } finally {
        setDesktopActivating("");
      }
    },
    [loadProfiles, showToast],
  );

  const confirmDesktop = useCallback(async () => {
    setDesktopActivating("__confirm__");
    setError("");
    try {
      await invoke("confirm_codex_desktop_profile");
      await loadProfiles();
      showToast(
        "The GPT app login was kept and its rollback copy was removed.",
      );
    } catch (reason) {
      const message = String(reason);
      await loadProfiles();
      setError(message);
    } finally {
      setDesktopActivating("");
    }
  }, [loadProfiles, showToast]);

  const rollbackDesktop = useCallback(async () => {
    setDesktopActivating("__rollback__");
    setError("");
    try {
      await invoke<DesktopActivationOutcome>("rollback_codex_desktop_profile");
      await loadProfiles();
      showToast("The previous GPT app login was restored.");
    } catch (reason) {
      const message = String(reason);
      await loadProfiles();
      setError(message);
    } finally {
      setDesktopActivating("");
    }
  }, [loadProfiles, showToast]);

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
          <h1>openProfiler</h1>
          <p>
            Manage local LLM profiles and choose the identity each provider uses
            next.
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
          {PROVIDER_FILTERS.map((item) => (
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

      <DesktopStatusPanel
        desktop={desktop}
        desktopActivating={desktopActivating}
        onConfirm={confirmDesktop}
        onRollback={rollbackDesktop}
      />

      {toast ? <output className="toast">{toast}</output> : null}

      <ProfileResults
        activating={activating}
        desktop={desktop}
        desktopActivating={desktopActivating}
        loading={loading}
        onActivate={activate}
        onActivateDesktop={activateDesktop}
        profiles={visibleProfiles}
      />

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

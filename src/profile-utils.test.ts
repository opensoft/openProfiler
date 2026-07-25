import { describe, expect, it } from "vitest";

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
import type { DesktopAppStatus, Profile } from "./types";

const profiles: Profile[] = [
  {
    provider: "codex",
    name: "work",
    email: "developer@company.example",
    family: "company",
    aliases: ["office"],
    profilePath: "company/work",
    status: "active",
    configured: true,
    credentialPresent: true,
    active: true,
    source: "manifest",
  },
  {
    provider: "claude",
    name: "personal",
    email: "",
    family: "personal",
    aliases: [],
    profilePath: "personal",
    status: "active",
    configured: true,
    credentialPresent: false,
    active: false,
    source: "profile-metadata",
  },
];

const desktop: DesktopAppStatus = {
  platformSupported: true,
  installed: true,
  running: false,
  fileActivationSupported: true,
  credentialStore: "file",
  eligibleProfilePaths: ["company/work"],
  activeProfilePaths: ["company/work"],
  rollbackAvailable: false,
  message: "ready",
};

describe("profile utilities", () => {
  it("filters by provider and searchable metadata", () => {
    expect(
      filterProfiles(profiles, "codex", "").map((item) => item.name),
    ).toEqual(["work"]);
    expect(
      filterProfiles(profiles, ALL_PROVIDERS, "CLAUDE").map(
        (item) => item.name,
      ),
    ).toEqual(["personal"]);
    expect(
      filterProfiles(profiles, ALL_PROVIDERS, "company.example").map(
        (item) => item.name,
      ),
    ).toEqual(["work"]);
  });

  it("uses a sentinel that cannot collide with a provider", () => {
    expect(filterProfiles(profiles, ALL_PROVIDERS, "")).toHaveLength(2);
  });

  it("describes active and credential states without credential contents", () => {
    expect(credentialLabel(profiles[0])).toBe("Active");
    expect(credentialLabel(profiles[1])).toBe("Login required");
  });

  it("formats provider labels", () => {
    expect(providerLabel("codex")).toBe("Codex");
    expect(providerLabel("claude")).toBe("Claude");
  });

  it("uses Tauri's camel-case command argument names", () => {
    expect(activationPayload(profiles[0])).toEqual({
      provider: "codex",
      profilePath: "company/work",
    });
    expect(desktopActivationPayload(profiles[0])).toEqual({
      profilePath: "company/work",
    });
  });

  it("maps desktop eligibility and active identity without tokens", () => {
    expect(desktopEligible(profiles[0], desktop)).toBe(true);
    expect(desktopActive(profiles[0], desktop)).toBe(true);
    expect(desktopEligible(profiles[1], desktop)).toBe(false);
    expect(desktopActive(profiles[1], desktop)).toBe(false);
  });
});

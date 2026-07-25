import { describe, expect, it } from "vitest";

import {
  ALL_PROVIDERS,
  credentialLabel,
  filterProfiles,
  providerLabel,
} from "./profile-utils";
import type { Profile } from "./types";

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
});

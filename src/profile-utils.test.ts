import { describe, expect, it } from "vitest";

import {
  ALL_FAMILIES,
  credentialLabel,
  filterProfiles,
  profileFamilies,
} from "./profile-utils";
import type { CodexProfile } from "./types";

const profiles: CodexProfile[] = [
  {
    name: "team-001",
    email: "developer@opensoft.example",
    family: "opensoft",
    aliases: ["team"],
    profilePath: "opensoft/team/team-001",
    status: "active",
    configured: true,
    credentialPresent: true,
    source: "manifest",
  },
  {
    name: "personal-001",
    email: "developer@example.com",
    family: "personal",
    aliases: [],
    profilePath: "personal/personal-001",
    status: "active",
    configured: false,
    credentialPresent: false,
    source: "manifest",
  },
];

describe("profile utilities", () => {
  it("sorts unique families", () => {
    expect(profileFamilies(profiles)).toEqual(["opensoft", "personal"]);
  });

  it("filters by family and searchable metadata", () => {
    expect(
      filterProfiles(profiles, "opensoft", "").map((item) => item.name),
    ).toEqual(["team-001"]);
    expect(
      filterProfiles(profiles, ALL_FAMILIES, "TEAM").map((item) => item.name),
    ).toEqual(["team-001"]);
    expect(
      filterProfiles(profiles, ALL_FAMILIES, "example.com").map(
        (item) => item.name,
      ),
    ).toEqual(["personal-001"]);
  });

  it("describes credential state without credential contents", () => {
    expect(credentialLabel(profiles[0])).toBe("Credential present");
    expect(credentialLabel(profiles[1])).toBe("Not materialized");
  });
});

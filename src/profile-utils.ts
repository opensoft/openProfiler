import type { CodexProfile } from "./types";

export const ALL_FAMILIES = "all";

export function profileFamilies(profiles: CodexProfile[]): string[] {
  return [...new Set(profiles.map((profile) => profile.family))].sort((a, b) =>
    a.localeCompare(b),
  );
}

export function filterProfiles(
  profiles: CodexProfile[],
  family: string,
  query: string,
): CodexProfile[] {
  const normalizedQuery = query.trim().toLocaleLowerCase();

  return profiles.filter((profile) => {
    if (family !== ALL_FAMILIES && profile.family !== family) {
      return false;
    }

    if (!normalizedQuery) {
      return true;
    }

    return [
      profile.name,
      profile.email,
      profile.family,
      profile.profilePath,
      ...profile.aliases,
    ].some((value) => value.toLocaleLowerCase().includes(normalizedQuery));
  });
}

export function credentialLabel(profile: CodexProfile): string {
  if (!profile.configured) {
    return "Not materialized";
  }

  return profile.credentialPresent ? "Credential present" : "Login required";
}

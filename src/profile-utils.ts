import type { Profile, Provider } from "./types";

export const ALL_PROVIDERS = "__all__";

export function filterProfiles(
  profiles: Profile[],
  provider: Provider | typeof ALL_PROVIDERS,
  query: string,
): Profile[] {
  const normalizedQuery = query.trim().toLowerCase();

  return profiles.filter((profile) => {
    if (provider !== ALL_PROVIDERS && profile.provider !== provider) {
      return false;
    }

    if (!normalizedQuery) {
      return true;
    }

    return [
      profile.provider,
      profile.name,
      profile.email,
      profile.family,
      profile.profilePath,
      ...profile.aliases,
    ].some((value) => value.toLowerCase().includes(normalizedQuery));
  });
}

export function credentialLabel(profile: Profile): string {
  if (profile.active) {
    return "Active";
  }
  if (!profile.configured) {
    return "Not materialized";
  }
  return profile.credentialPresent ? "Ready" : "Login required";
}

export function activationPayload(profile: Profile) {
  return {
    provider: profile.provider,
    profilePath: profile.profilePath,
  };
}

export function providerLabel(provider: Provider): string {
  return provider === "codex" ? "Codex" : "Claude";
}

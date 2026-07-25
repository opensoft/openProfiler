export type Provider = "codex" | "claude";

export interface Profile {
  provider: Provider;
  name: string;
  email: string;
  family: string;
  aliases: string[];
  profilePath: string;
  status: string;
  configured: boolean;
  credentialPresent: boolean;
  active: boolean;
  source: "manifest" | "profile-metadata" | "profile-directory";
}

export interface ProviderStore {
  provider: Provider;
  manifestPath: string;
  profilesHome: string;
  activeHome: string;
  issues: string[];
}

export interface ProfileInventory {
  stores: ProviderStore[];
  profiles: Profile[];
}

export interface ActivationResult {
  provider: Provider;
  profile: string;
  restartRequired: boolean;
}

export interface DesktopAppStatus {
  platformSupported: boolean;
  installed: boolean;
  running: boolean;
  fileActivationSupported: boolean;
  credentialStore: string;
  eligibleProfilePaths: string[];
  activeProfilePaths: string[];
  rollbackAvailable: boolean;
  message: string;
}

export interface DesktopActivationOutcome {
  profile: string;
  outgoingProfilesUpdated: number;
  rollbackAvailable: boolean;
  relaunched: boolean;
}

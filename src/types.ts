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
  activeCredentialPath: string;
  restartRequired: boolean;
}

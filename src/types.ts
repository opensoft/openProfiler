export type CodexAction = "login" | "status" | "launch" | "logout";

export interface CodexProfile {
  name: string;
  email: string;
  family: string;
  aliases: string[];
  profilePath: string;
  status: string;
  configured: boolean;
  credentialPresent: boolean;
  source: "manifest" | "profile-metadata";
}

export interface ProfileInventory {
  manifestPath: string;
  profilesHome: string;
  profiles: CodexProfile[];
}

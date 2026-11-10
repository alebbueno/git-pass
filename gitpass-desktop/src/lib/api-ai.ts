import { invoke } from "@tauri-apps/api/core";

export type AiStatus = {
  enabled: boolean;
  provider: string;
  model: string;
  responseLocale: string;
  consentVersion: number;
  openaiKeyConfigured: boolean;
  anthropicKeyConfigured: boolean;
  googleKeyConfigured: boolean;
  curatedModels: string[];
  recommendedModel: string;
  configPath: string;
};

export const aiApi = {
  status: () => invoke<AiStatus>("ai_status"),
  setEnabled: (enabled: boolean, consentVersion: number) =>
    invoke<AiStatus>("ai_set_enabled", { enabled, consentVersion }),
  setKey: (provider: string, key: string) => invoke<AiStatus>("ai_set_key", { provider, key }),
  testKey: (provider: string) => invoke<string>("ai_test_key", { provider }),
  removeKey: (provider: string) => invoke<AiStatus>("ai_remove_key", { provider }),
  setPreferences: (patch: { model?: string; responseLocale?: string }) =>
    invoke<AiStatus>("ai_set_preferences", { patch }),
};

export const OPENAI_BILLING_URL = "https://platform.openai.com/settings/organization/billing";

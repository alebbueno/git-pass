import { invoke } from "@tauri-apps/api/core";

/** Resposta de `accounts_list` (campos usados pelo shell e pelas vistas); `AccountInfo` em api-accounts.ts tem o tipo completo. */
export type AccountRow = {
  id: string;
  login: string | null;
  label: string | null;
  name?: string | null;
  email?: string | null;
  avatar_url?: string | null;
  auth: string;
  has_token: boolean;
};

export type BindingRow = {
  id: string;
  account_id: string;
  account_login: string | null;
  root_path: string;
  github_owner: string | null;
  github_repo: string | null;
};

export type OauthConfigStatus = {
  ok: boolean;
  message: string;
  oauth_env_path: string;
  client_id_configured: boolean;
};

export type DeviceCodeStart = {
  client_id: string;
  user_code: string;
  verification_uri: string;
  device_code: string;
  interval_secs: number;
  expires_in_secs: number;
};

export const api = {
  configPath: () => invoke<string>("get_config_path"),
  oauthStatus: () => invoke<OauthConfigStatus>("oauth_config_status"),
  accounts: () => invoke<AccountRow[]>("accounts_list"),
  bindings: () => invoke<BindingRow[]>("list_bindings"),
  commitLog: (rootPath: string, limit = 40) =>
    invoke<string[]>("repo_commit_log", { rootPath, limit }),
  pingDaemon: () => invoke<string>("ipc_ping_gitpassd"),
  keychainHint: () => invoke<string>("open_keychain_docs"),
  openDataFolder: () => invoke<void>("open_application_support"),
  openExternal: (url: string) => invoke<void>("open_external_url", { url }),
};

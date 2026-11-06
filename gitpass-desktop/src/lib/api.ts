import { invoke } from "@tauri-apps/api/core";

export type AccountRow = {
  id: string;
  login: string | null;
  label: string | null;
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

export type GithubRepoSummary = {
  full_name: string;
  name: string;
  private: boolean;
  html_url: string;
  pushed_at: string | null;
  default_branch: string | null;
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

export type DevicePollUi =
  | { status: "pending" }
  | { status: "slow_down"; interval_secs: number }
  | { status: "done"; message: string };

export const api = {
  configPath: () => invoke<string>("get_config_path"),
  oauthStatus: () => invoke<OauthConfigStatus>("oauth_config_status"),
  accounts: () => invoke<AccountRow[]>("list_accounts"),
  bindings: () => invoke<BindingRow[]>("list_bindings"),
  commitLog: (rootPath: string, limit = 40) =>
    invoke<string[]>("repo_commit_log", { rootPath, limit }),
  cloudRepos: (accountId: string) =>
    invoke<GithubRepoSummary[]>("github_cloud_repos_for_account", { accountId }),
  deviceStart: () => invoke<DeviceCodeStart>("github_device_oauth_start"),
  devicePoll: (clientId: string, deviceCode: string) =>
    invoke<DevicePollUi>("github_device_oauth_poll", { clientId, deviceCode }),
  /** Abre o seletor de pastas nativo e cria o vínculo; rejeita se o utilizador cancelar ou o origin não for GitHub. */
  bindRepoDialog: (accountId: string) => invoke<void>("bind_repo_dialog", { accountId }),
  pingDaemon: () => invoke<string>("ipc_ping_gitpassd"),
  keychainHint: () => invoke<string>("open_keychain_docs"),
  openDataFolder: () => invoke<void>("open_application_support"),
  openExternal: (url: string) => invoke<void>("open_external_url", { url }),
};

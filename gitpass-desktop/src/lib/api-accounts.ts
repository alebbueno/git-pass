import { invoke } from "@tauri-apps/api/core";
import type { DeviceCodeStart, OauthConfigStatus } from "./api";

export type AccountInfo = {
  id: string;
  host: "github" | "bitbucket";
  login: string | null;
  label: string | null;
  name: string | null;
  email: string | null;
  avatar_url: string | null;
  auth: string;
  has_token: boolean;
  /** `null` até `accountsApi.check` correr (a listagem não usa a rede). */
  token_ok: boolean | null;
  scopes: string[];
  missing_scopes: string[];
  repos_count: number;
};

export type AccountDevicePoll =
  | { status: "pending" }
  | { status: "slow_down"; interval_secs: number }
  | {
      status: "done";
      message: string;
      account_id: string;
      login: string;
      reconnected: boolean;
      warning: string | null;
    };

export type TokenState = "ok" | "revoked" | "missing" | "offline" | "error";

export type AccountCheck = {
  account_id: string;
  state: TokenState;
  scopes: string[];
  missing_scopes: string[];
  needs_reconnect: boolean;
  message: string;
};

export type EmailOption = {
  email: string;
  primary: boolean;
  noreply: boolean;
  selected: boolean;
};

export type AccountEmails = {
  emails: EmailOption[];
  current: string | null;
  needs_reconnect: boolean;
};

export type IdentityUpdate = {
  account: AccountInfo;
  repos_updated: number;
  repos_failed: string[];
};

export const accountsApi = {
  list: () => invoke<AccountInfo[]>("accounts_list"),
  deviceStart: () => invoke<DeviceCodeStart>("accounts_device_start"),
  devicePoll: (clientId: string, deviceCode: string) =>
    invoke<AccountDevicePoll>("accounts_device_poll", { clientId, deviceCode }),
  refreshProfile: (accountId: string) => invoke<AccountInfo>("accounts_refresh_profile", { accountId }),
  setLabel: (accountId: string, label: string) => invoke<AccountInfo>("accounts_set_label", { accountId, label }),
  emails: (accountId: string) => invoke<AccountEmails>("accounts_emails", { accountId }),
  setEmail: (accountId: string, email: string) => invoke<IdentityUpdate>("accounts_set_email", { accountId, email }),
  remove: (accountId: string) => invoke<{ removed_bindings: number }>("accounts_remove", { accountId }),
  check: (accountId: string) => invoke<AccountCheck>("accounts_check", { accountId }),
  bitbucketConnect: (username: string, appPassword: string, label?: string | null) =>
    invoke<{ account: AccountInfo; message: string }>("accounts_bitbucket_connect", {
      username,
      appPassword,
      label: label ?? null,
    }),
  bitbucketOauthStatus: () => invoke<OauthConfigStatus>("bitbucket_oauth_config_status"),
  bitbucketOauthConnect: () =>
    invoke<{ account: AccountInfo; message: string }>("accounts_bitbucket_oauth_connect"),
};

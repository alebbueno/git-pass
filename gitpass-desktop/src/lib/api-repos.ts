import { invoke } from "@tauri-apps/api/core";

export type IdentityState = "correct" | "different" | "not_set";

export type SuggestionReason = "owner_login" | "same_owner_binding" | "only_account" | "push_access";

export type RepoIdentityRow = {
  /** `null` quando a pasta já não existe. */
  state: IdentityState | null;
  local_name: string | null;
  local_email: string | null;
  expected_name: string | null;
  expected_email: string | null;
};

export type RepoRow = {
  id: string;
  account_id: string;
  account_login: string | null;
  account_label: string | null;
  account_missing: boolean;
  root_path: string;
  github_owner: string | null;
  github_repo: string | null;
  /** Sempre `https://github.com/owner/repo`, sem credenciais. */
  remote_url: string | null;
  remote_has_credentials: boolean;
  exists: boolean;
  branch: string | null;
  detached_at: string | null;
  identity: RepoIdentityRow;
};

export type BindCandidate = { account_id: string; can_push: boolean | null };

export type BindOutcome =
  | { status: "cancelled" }
  | { status: "bound"; repo: RepoRow; reason: SuggestionReason | null }
  | {
      status: "needs_account";
      root_path: string;
      github_owner: string;
      github_repo: string;
      suggested_account_id: string | null;
      candidates: BindCandidate[];
    };

export type BindItemResult = { path: string; repo: RepoRow | null; error: string | null };

export type DiscoveredRepo = {
  root_path: string;
  github_owner: string;
  github_repo: string;
  remote_url: string;
  suggested_account_id: string | null;
  suggestion_reason: SuggestionReason | null;
};

export type DiscoverResult = {
  repos: DiscoveredRepo[];
  roots: string[];
  truncated: boolean;
  elapsed_ms: number;
};

export const reposApi = {
  list: () => invoke<RepoRow[]>("repos_list"),
  /** Seletor de pastas nativo; com `accountId = null` a conta é escolhida automaticamente (ou pede confirmação). */
  bindDialog: (accountId: string | null = null) => invoke<BindOutcome>("repos_bind_dialog", { accountId }),
  bindPath: (path: string, accountId: string) => invoke<RepoRow>("repos_bind_path", { path, accountId }),
  bindMany: (items: { path: string; account_id: string }[]) =>
    invoke<BindItemResult[]>("repos_bind_many", { items }),
  unbind: (bindingId: string, resetIdentity: boolean) =>
    invoke<void>("repos_unbind", { bindingId, resetIdentity }),
  changeAccount: (bindingId: string, accountId: string) =>
    invoke<RepoRow>("repos_change_account", { bindingId, accountId }),
  fixIdentity: (bindingId: string) => invoke<RepoRow>("repos_fix_identity", { bindingId }),
  discover: (roots: string[] | null = null) => invoke<DiscoverResult>("repos_discover", { roots }),
  /** `null` quando o utilizador cancela. */
  pickFolder: () => invoke<string | null>("repos_pick_folder"),
  openInFinder: (path: string) => invoke<void>("repos_open_in_finder", { path }),
  openInTerminal: (path: string) => invoke<void>("repos_open_in_terminal", { path }),
};

import { invoke } from "@tauri-apps/api/core";

export type RepoPermissions = {
  admin: boolean;
  maintain: boolean;
  push: boolean;
  triage: boolean;
  pull: boolean;
};

/** Repositório GitHub com permissões e (se existir) o clone local vinculado à mesma conta. */
export type CloudRepo = {
  full_name: string;
  name: string;
  private: boolean;
  html_url: string;
  pushed_at: string | null;
  default_branch: string | null;
  permissions: RepoPermissions;
  owner_login: string;
  /** "User" ou "Organization". */
  owner_type: string;
  fork: boolean;
  archived: boolean;
  description: string | null;
  clone_url: string;
  local_binding_id: string | null;
  local_path: string | null;
};

export type ClonePhase =
  | "starting"
  | "creating"
  | "counting"
  | "compressing"
  | "receiving"
  | "resolving"
  | "checkout"
  | "binding";

/** Payload do evento `gitpass-clone-progress`. */
export type CloneProgressPayload = {
  full_name: string;
  phase: ClonePhase | string;
  percent: number;
};

export const CLONE_PROGRESS_EVENT = "gitpass-clone-progress";

export type PrInboxItem = {
  number: number;
  title: string;
  html_url: string;
  repo_full_name: string;
  updated_at: string;
  kind: "review_requested" | "authored" | string;
  state: string;
};

export const apiCloud = {
  /** Repositórios da conta com permissões; `local_binding_id` preenchido quando já há clone vinculado. */
  repos: (accountId: string) => invoke<CloudRepo[]>("cloud_repos", { accountId }),
  /**
   * Clona `owner/repo` e vincula à conta. `destParent` é a pasta que vai conter a pasta do repositório
   * (omitido → `~/GitHub/<owner>/<repo>`). Devolve o caminho do clone.
   */
  clone: (accountId: string, fullName: string, destParent?: string | null) =>
    invoke<string>("cloud_clone", { accountId, fullName, destParent: destParent ?? null }),
  /**
   * Cria um repositório vazio no GitHub (conta pessoal) e clona na pasta escolhida.
   * `repoName` é só o nome do repositório, sem `dono/`.
   */
  createAndClone: (
    accountId: string,
    repoName: string,
    isPrivate: boolean,
    description?: string | null,
    destParent?: string | null
  ) =>
    invoke<string>("cloud_create_and_clone", {
      accountId,
      repoName,
      private: isPrivate,
      description: description ?? null,
      destParent: destParent ?? null,
    }),
  /** Seletor de pastas nativo; `null` se o utilizador cancelar. */
  chooseFolder: () => invoke<string | null>("cloud_choose_folder"),
  prInbox: (accountId: string, refresh = true) =>
    invoke<PrInboxItem[]>("cloud_pr_inbox", { accountId, refresh }),
};

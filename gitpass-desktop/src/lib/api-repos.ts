import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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
  /** Resumo local (sem rede, desde o último fetch). `null` quando não deu para calcular. */
  sync: RepoSyncSummary | null;
};

export type RepoSyncSummary = {
  /** Commits locais que ainda não estão no GitHub. */
  ahead: number;
  /** Commits no GitHub que ainda não estão aqui (desde o último fetch). */
  behind: number;
  has_upstream: boolean;
  /** Há alterações não commitadas (staged, unstaged ou untracked). */
  dirty: boolean;
};

// ---------- Enviar commits (push validado) ----------

export type CheckSeverity = "error" | "warning" | "info";

/** `error` bloqueia o envio; `warning` exige `acknowledgeWarnings`; `info` só informa. */
export type PushCheck = {
  code:
    | "author_mismatch"
    | "committer_mismatch"
    | "large_file"
    | "file_too_large"
    | "secret_suspected"
    | "behind_remote"
    | "no_push_permission"
    | "account_disconnected"
    | "detached_head"
    | "no_remote"
    | "dirty_worktree"
    | "merge_commit"
    | "fetch_failed"
    | "nothing_to_push"
    // Prévia de merge / cherry-pick:
    | "merge_conflict"
    | "target_dirty"
    | "target_behind_remote"
    | "already_in_target"
    | "source_pushed";
  severity: CheckSeverity;
  message: string;
  /** Commit afetado, quando o problema é de um commit específico. */
  sha: string | null;
  /** Arquivo afetado (arquivos grandes, possível segredo). */
  path: string | null;
};

export type OutgoingCommit = {
  sha: string;
  short_sha: string;
  subject: string;
  body: string;
  author_name: string;
  author_email: string;
  /** ISO 8601. */
  authored_at: string;
  committer_name: string;
  committer_email: string;
  files_changed: number;
  insertions: number;
  deletions: number;
  is_merge: boolean;
  /** Checks deste commit (subconjunto de `OutgoingState.checks` com `sha` igual). */
  checks: PushCheck[];
};

export type OutgoingState = {
  binding_id: string;
  branch: string | null;
  remote_name: string | null;
  /** Ex.: `origin/main`. `null` quando o branch ainda não existe no GitHub. */
  upstream: string | null;
  /** `https://github.com/owner/repo/tree/<branch>`, sem credenciais. */
  remote_branch_url: string | null;
  ahead: number;
  behind: number;
  /** `true` quando o estado veio de um fetch feito agora. */
  fetched: boolean;
  dirty: { staged: number; unstaged: number; untracked: number };
  /** Mais novo primeiro. No máximo 200. */
  commits: OutgoingCommit[];
  truncated: boolean;
  /** Todos os checks (do repositório e dos commits). */
  checks: PushCheck[];
  /** HEAD atual; enviar de volta em `push`/`fixAuthors` para garantir que nada mudou. */
  head_sha: string | null;
  /** Nenhum check `error`. */
  can_push: boolean;
  has_warnings: boolean;
  /** Dá para reescrever autor/committer dos commits não enviados (árvore limpa, sem merges). */
  can_fix_authors: boolean;
  /** Dá para trazer os commits do GitHub com rebase (árvore limpa, upstream existe, behind > 0). */
  can_pull: boolean;
};

export type CommitFile = {
  path: string;
  /** Caminho antigo em renomeações. */
  old_path: string | null;
  status: "added" | "modified" | "deleted" | "renamed" | "copied" | "type_changed";
  insertions: number | null;
  deletions: number | null;
  binary: boolean;
  size_bytes: number | null;
};

export type PushResult = {
  pushed: number;
  branch: string;
  /** O branch foi criado no GitHub neste envio (upstream configurado). */
  created_upstream: boolean;
  remote_branch_url: string | null;
  /** Saída resumida e sem credenciais, para "Ver detalhes". */
  summary: string;
};

export type PushProgress = {
  binding_id: string;
  phase: "validating" | "connecting" | "counting" | "compressing" | "writing" | "resolving" | "done";
  percent: number | null;
};

// ---------- Branches e "levar commits para outro branch" (merge / cherry-pick) ----------

export type BranchInfo = {
  name: string;
  /** Branch atual da pasta. */
  current: boolean;
  /** Branch padrão do GitHub (ex.: `main`), pelo `origin/HEAD` ou pela API. */
  is_default: boolean;
  /** Ex.: `origin/main`. */
  upstream: string | null;
  /** Em relação ao próprio upstream. `null` sem upstream. */
  ahead: number | null;
  behind: number | null;
  /** Aberto em outra pasta (git worktree); `null` se não. */
  worktree_path: string | null;
  head_sha: string;
  /** Data do último commit, ISO 8601. */
  last_commit_at: string;
  last_commit_subject: string;
  /** Existe só no GitHub (ainda não há branch local). */
  remote_only: boolean;
};

export type MergeMode = "merge" | "cherry_pick";

/** Como o resultado vai ser aplicado no branch de destino. */
export type MergeStrategy =
  /** O destino só avança (sem commit novo). */
  | "fast_forward"
  /** Commit de merge novo (ou commits copiados, no cherry-pick). */
  | "new_commits"
  /** Nada a fazer: os commits já estão no destino. */
  | "up_to_date";

export type MergePreview = {
  binding_id: string;
  source: string;
  target: string;
  mode: MergeMode;
  /** Commits do `source` que ainda não estão no `target`, mais novo primeiro (máx. 200). */
  commits: OutgoingCommit[];
  /** Para `cherry_pick`: os SHAs escolhidos, na ordem em que serão aplicados (mais antigo primeiro). */
  selected: string[];
  strategy: MergeStrategy;
  /** Arquivos com conflito (prévia sem mexer na pasta). Vazio = sem conflito. */
  conflicts: string[];
  files_changed: number;
  insertions: number;
  deletions: number;
  /** `error` bloqueia (ex.: conflito, pasta de destino com alterações sem commit); `warning` exige confirmação. */
  checks: PushCheck[];
  can_apply: boolean;
  /** Valores para enviar em `apply`, garantindo que nada mudou desde a prévia. */
  source_sha: string;
  target_sha: string;
  /** O merge será feito na pasta do repositório (destino é o branch atual) ou sem tocar em nenhuma pasta. */
  applies_in_worktree: boolean;
};

export type MergeResult = {
  target: string;
  /** Novo HEAD do destino. */
  new_target_sha: string;
  strategy: MergeStrategy;
  /** Commits criados no destino (merge commit ou cópias do cherry-pick). */
  created: number;
  /** Commits do destino que ainda não estão no GitHub, para oferecer "Enviar agora". */
  ahead_after: number;
};

export const PUSH_PROGRESS_EVENT = "repos-push-progress";
/** Emitido depois de push, pull ou correção de autor (payload: `binding_id`). */
export const OUTGOING_CHANGED_EVENT = "repos-outgoing-changed";

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

/** Grupo de cada projeto em «Mapear pasta». */
export type MapStatus = "bound" | "match" | "read_only" | "not_found" | "other_remote" | "no_remote";

/** Permissão efetiva da conta no repositório, da maior para a menor. */
export type RepoPermission = "admin" | "maintain" | "push" | "triage" | "pull";

export type MapCandidate = { account_id: string; permission: RepoPermission };

export type MappedProject = {
  root_path: string;
  status: MapStatus;
  remote_name: string | null;
  /** GitHub: `https://github.com/owner/repo`. Outros serviços: o remote sem credenciais. */
  remote_url: string | null;
  github_owner: string | null;
  github_repo: string | null;
  /** Contas com acesso, na ordem das configurações. */
  candidates: MapCandidate[];
  suggested_account_id: string | null;
  binding_id: string | null;
  bound_account_id: string | null;
  /** Só em `bound`: `false` quando outra conta tem permissão maior; `null` quando não deu para saber. */
  bound_matches_suggestion: boolean | null;
};

export type MapAccountStatus = { account_id: string; ok: boolean; error: string | null; repo_count: number };

export type MapResult = {
  root: string;
  items: MappedProject[];
  accounts: MapAccountStatus[];
  truncated: boolean;
  limit_reached: boolean;
  max_projects: number;
  elapsed_ms: number;
};

export type MapProgress = { phase: "scanning" | "github" | "matching"; found: number };

export const MAP_PROGRESS_EVENT = "repos-map-progress";

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
  /** `null` quando o usuário cancela. */
  pickFolder: () => invoke<string | null>("repos_pick_folder"),
  /** Procura os projetos Git em `root` e cruza com as contas conectadas. Progresso em `onMapProgress`. */
  mapFolder: (root: string) => invoke<MapResult>("repos_map_folder", { root }),
  onMapProgress: (cb: (p: MapProgress) => void): Promise<UnlistenFn> =>
    listen<MapProgress>(MAP_PROGRESS_EVENT, (e) => cb(e.payload)),
  /**
   * Commits locais ainda não enviados e as validações. Com `fetch`, atualiza do GitHub antes (usa a conta vinculada).
   * `branch = null` usa o branch atual; com um nome, valida esse branch local (pode não ser o atual).
   */
  outgoing: (bindingId: string, fetch: boolean, branch: string | null = null) =>
    invoke<OutgoingState>("repos_outgoing", { bindingId, fetch, branch }),
  commitFiles: (bindingId: string, sha: string) => invoke<CommitFile[]>("repos_commit_files", { bindingId, sha }),
  /**
   * Falha se o branch mudou desde `expectedHead`, se houver check `error`, ou se houver `warning` sem `acknowledgeWarnings`.
   * `branch = null` envia o branch atual; com um nome, envia esse branch local sem trocar de branch na pasta.
   */
  push: (bindingId: string, expectedHead: string, acknowledgeWarnings: boolean, branch: string | null = null) =>
    invoke<PushResult>("repos_push", { bindingId, expectedHead, acknowledgeWarnings, branch }),
  /** Branches locais (e os que só existem no GitHub, desde o último fetch). Com `fetch`, atualiza antes. */
  branches: (bindingId: string, fetch: boolean) => invoke<BranchInfo[]>("repos_branches", { bindingId, fetch }),
  /** Troca o branch da pasta. Recusa com alterações sem commit. Branch `remote_only` é criado a partir do GitHub. */
  checkout: (bindingId: string, branch: string) => invoke<RepoRow>("repos_checkout", { bindingId, branch }),
  /** Prévia, sem alterar nada. `commits` só vale para `cherry_pick` (SHAs do `source`; vazio = todos). */
  mergePreview: (bindingId: string, source: string, target: string, mode: MergeMode, commits: string[]) =>
    invoke<MergePreview>("repos_merge_preview", { bindingId, source, target, mode, commits }),
  /** Aplica a prévia. Falha se `source`/`target` mudaram desde a prévia ou se surgir conflito (nada fica pela metade). */
  mergeApply: (
    bindingId: string,
    source: string,
    target: string,
    mode: MergeMode,
    commits: string[],
    sourceSha: string,
    targetSha: string,
  ) => invoke<MergeResult>("repos_merge_apply", { bindingId, source, target, mode, commits, sourceSha, targetSha }),
  /** Reescreve autor e committer dos commits não enviados com a identidade da conta vinculada. */
  fixAuthors: (bindingId: string, expectedHead: string) =>
    invoke<OutgoingState>("repos_fix_authors", { bindingId, expectedHead }),
  /** `git pull --rebase` com a conta vinculada; aborta e restaura se houver conflito. */
  pull: (bindingId: string) => invoke<OutgoingState>("repos_pull", { bindingId }),
  onPushProgress: (cb: (p: PushProgress) => void): Promise<UnlistenFn> =>
    listen<PushProgress>(PUSH_PROGRESS_EVENT, (e) => cb(e.payload)),
  onOutgoingChanged: (cb: (bindingId: string) => void): Promise<UnlistenFn> =>
    listen<string>(OUTGOING_CHANGED_EVENT, (e) => cb(e.payload)),
  openInFinder: (path: string) => invoke<void>("repos_open_in_finder", { path }),
  openInTerminal: (path: string) => invoke<void>("repos_open_in_terminal", { path }),
};

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ---------- Repo atual (menu bar): fixado > foco > recente ----------

/** `pinned`: fixado pelo usuário. `focus`: app em primeiro plano (Accessibility). `recent`: último validado por hook/CLI. */
export type CurrentRepoSource = "pinned" | "focus" | "recent";

/** Resultado da última validação feita pelo GitPass nesse repo. */
export type ValidationVerdict = "ok" | "blocked" | "unconfirmed";

export type CurrentRepo = {
  binding_id: string | null;
  root_path: string;
  /** `owner/repo` no GitHub, quando conhecido. */
  full_name: string | null;
  account_id: string | null;
  source: CurrentRepoSource;
  /** ISO 8601 do último evento que tornou este repo o atual. */
  at: string;
  verdict: ValidationVerdict | null;
  /** Código `GP-*` da última validação, se houver. */
  code: string | null;
  /** Mensagem curta, sem segredos. */
  message: string | null;
};

export type CurrentRepoState = {
  current: CurrentRepo | null;
  pinned_binding_id: string | null;
  /** Até 10 repos validados recentemente, mais novo primeiro. */
  recent: CurrentRepo[];
  /** O macOS deu permissão de Acessibilidade (para detectar o app em foco). */
  accessibility_granted: boolean;
};

// ---------- Modo emergência (DL-015/016) ----------

export type EmergencyState = {
  active: boolean;
  /** ISO 8601; `null` quando inativo. */
  until: string | null;
  started_at: string | null;
  reason: string | null;
};

// ---------- Auditoria (local, sem segredos) ----------

export type AuditKind =
  | "emergency_on"
  | "emergency_off"
  | "emergency_expired"
  | "commit_allowed_emergency"
  | "blocked"
  | "unconfirmed"
  | "ci_bypass";

export type AuditEntry = {
  at: string;
  kind: AuditKind;
  repo: string | null;
  code: string | null;
  message: string;
};

// ---------- Diagnóstico (doctor) ----------

export type DiagnosticState = "ok" | "warning" | "error" | "info";

export type DiagnosticItem = {
  /** Ex.: `daemon`, `hooks`, `credentials`, `git`, `keychain`, `ssh`, `bindings`, `macos`, `accessibility`. */
  id: string;
  label: string;
  state: DiagnosticState;
  detail: string;
  code: string | null;
  /** Ação sugerida que a UI pode oferecer: `repair_integration`, `restart_daemon`, `open_accessibility`, `open_accounts`, `open_repos`. */
  action: string | null;
};

export type Diagnostics = {
  generated_at: string;
  app_version: string;
  macos_version: string | null;
  git_version: string | null;
  items: DiagnosticItem[];
};

export const CURRENT_REPO_CHANGED_EVENT = "gitpass-current-repo-changed";
export const EMERGENCY_CHANGED_EVENT = "gitpass-emergency-changed";

export const systemApi = {
  currentRepo: () => invoke<CurrentRepoState>("system_current_repo"),
  /** `null` desafixa. */
  pinRepo: (bindingId: string | null) => invoke<CurrentRepoState>("system_pin_repo", { bindingId }),
  emergencyState: () => invoke<EmergencyState>("system_emergency_state"),
  /** `minutes` entre 5 e 240. Permite só commits locais; push continua bloqueado. */
  emergencyStart: (minutes: number, reason: string) =>
    invoke<EmergencyState>("system_emergency_start", { minutes, reason }),
  emergencyStop: () => invoke<EmergencyState>("system_emergency_stop"),
  audit: (limit: number) => invoke<AuditEntry[]>("system_audit", { limit }),
  /** `probeTokens: false` evita ler o Keychain (contador da barra lateral). Na tela Diagnóstico use o padrão. */
  diagnostics: (opts?: { probeTokens?: boolean }) =>
    invoke<Diagnostics>("system_diagnostics", { probeTokens: opts?.probeTokens ?? true }),
  /** Abre "Salvar como" e grava um ZIP com logs redigidos + manifesto. `null` se cancelado. */
  exportLogs: () => invoke<string | null>("system_export_logs"),
  openAccessibilitySettings: () => invoke<void>("system_open_accessibility_settings"),
  onCurrentRepoChanged: (cb: (s: CurrentRepoState) => void): Promise<UnlistenFn> =>
    listen<CurrentRepoState>(CURRENT_REPO_CHANGED_EVENT, (e) => cb(e.payload)),
  onEmergencyChanged: (cb: (s: EmergencyState) => void): Promise<UnlistenFn> =>
    listen<EmergencyState>(EMERGENCY_CHANGED_EVENT, (e) => cb(e.payload)),
};

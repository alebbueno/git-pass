import "./settings.css";
import { api, type OauthConfigStatus } from "../lib/api";
import {
  integrationApi,
  onIntegrationChanged,
  type CheckState,
  type ComponentStatus,
  type IntegrationReport,
  type IntegrationStatus,
} from "../lib/api-integration";
import { errMessage, esc, showToast, withBusy } from "../lib/dom";
import { brandSymbol, icon } from "../lib/icons";
import { setTheme, storedTheme, type ThemeMode } from "../lib/theme";
import type { MountView } from "../lib/view";

const THEME_EVENT = "gitpass:theme-changed";

const CHECK_BADGES: Record<CheckState, { cls: string; sym: "check" | "alert" | "x"; label: string }> = {
  ok: { cls: "success", sym: "check", label: "Ativo" },
  missing: { cls: "error", sym: "x", label: "Faltando" },
  outdated: { cls: "warning", sym: "alert", label: "Desatualizado" },
  conflict: { cls: "warning", sym: "alert", label: "Conflito" },
  error: { cls: "error", sym: "x", label: "Erro" },
};

type CheckId = "credentials" | "hooks" | "daemon";

const CHECKS: { id: CheckId; label: string; current: string; previous: string }[] = [
  { id: "credentials", label: "Credenciais do GitHub", current: "Configurado agora", previous: "Substituído (salvo para restaurar)" },
  { id: "hooks", label: "Verificação do Git", current: "Pasta de verificação atual", previous: "Também roda as verificações em" },
  { id: "daemon", label: "Serviço em segundo plano", current: "Atual", previous: "Anterior" },
];

const THEME_OPTIONS: { mode: ThemeMode; label: string; icon: "monitor" | "sun" | "moon" }[] = [
  { mode: "system", label: "Auto", icon: "monitor" },
  { mode: "light", label: "Claro", icon: "sun" },
  { mode: "dark", label: "Escuro", icon: "moon" },
];

const attr = esc;

function copyButton(value: string, what: string): string {
  return `<button type="button" class="btn ghost icon-only settings-copy" data-copy="${attr(value)}" aria-label="Copiar" title="Copiar ${attr(what)}">${icon("copy", 16)}</button>`;
}

function pathBox(value: string, what: string): string {
  return `<div class="settings-path"><span class="settings-path-text mono small">${esc(value)}</span>${copyButton(value, what)}</div>`;
}

function previewTile(mode: "light" | "dark", label: string): string {
  return `
    <button type="button" class="settings-preview-btn" data-theme-pick="${mode}" aria-pressed="false" aria-label="Tema ${label}">
      <span class="settings-preview settings-preview-${mode}" aria-hidden="true">
        <span class="settings-preview-header">
          <span class="settings-preview-mark">${brandSymbol()}</span>
          <span class="settings-preview-brand">RepoID</span>
        </span>
        <span class="settings-preview-row">
          <span class="settings-preview-name mono">checkout-web</span>
          <span class="profile-chip profile-iris"><span class="profile-chip-initials">TR</span><span class="profile-chip-name">Trabalho</span></span>
        </span>
        <span class="settings-preview-row">
          <span class="settings-preview-name mono">portfolio</span>
          <span class="profile-chip profile-aqua"><span class="profile-chip-initials">PS</span><span class="profile-chip-name">Pessoal</span></span>
        </span>
      </span>
      <span class="settings-preview-caption">${icon(mode === "light" ? "sun" : "moon", 14)}${label}</span>
    </button>`;
}

export const mount: MountView = (root) => {
  let alive = true;

  root.innerHTML = `
    <header class="view-header">
      <div>
        <h1 class="view-title">Configurações</h1>
        <p class="view-sub">Aparência, conexões e dados locais do GitPass.</p>
      </div>
    </header>

    <div class="settings-page">
      <section aria-labelledby="settings-h-appearance">
        <h2 class="section-label settings-group-label" id="settings-h-appearance">Aparência</h2>
        <div class="panel settings-group">
          <div class="settings-row">
            <div class="settings-row-text">
              <div class="settings-row-label">Tema</div>
              <div class="settings-row-desc">Auto segue a configuração do Mac.</div>
            </div>
            <div class="settings-row-control">
              <div class="segmented" role="group" aria-label="Tema">
                ${THEME_OPTIONS.map(
                  (o) => `<button type="button" data-theme-mode="${o.mode}" aria-pressed="false">${icon(o.icon, 14)}<span>${o.label}</span></button>`
                ).join("")}
              </div>
            </div>
          </div>
          <div class="settings-row settings-row-stack">
            <div class="settings-previews">
              ${previewTile("light", "Claro")}
              ${previewTile("dark", "Escuro")}
            </div>
          </div>
        </div>
      </section>

      <section aria-labelledby="settings-h-oauth">
        <h2 class="section-label settings-group-label" id="settings-h-oauth">GitHub</h2>
        <div class="panel settings-group">
          <div class="settings-row">
            <div class="settings-row-text">
              <div class="settings-row-label">Conexão com o GitHub</div>
              <div class="settings-row-desc" id="settings-oauth-desc">Verificando…</div>
            </div>
            <div class="settings-row-control" id="settings-oauth-badge"><span class="spinner" role="status" aria-label="Carregando"></span></div>
          </div>
        </div>
      </section>

      <section aria-labelledby="settings-h-integration">
        <h2 class="section-label settings-group-label" id="settings-h-integration">Integração com o Git</h2>
        <div class="panel settings-group" id="settings-integration" aria-live="polite">
          <div class="settings-row">
            <div class="settings-row-text">
              <div class="settings-row-desc">Verificando a integração…</div>
            </div>
            <span class="spinner" role="status" aria-label="Verificando"></span>
          </div>
        </div>
      </section>

      <section aria-labelledby="settings-h-data">
        <h2 class="section-label settings-group-label" id="settings-h-data">Dados e segurança</h2>
        <div class="panel settings-group">
          <div class="settings-row settings-row-stack">
            <div class="settings-row-label">Arquivo de configuração</div>
            <div id="settings-config-path"><span class="muted small">Carregando…</span></div>
          </div>
          <div class="settings-row">
            <div class="settings-row-text">
              <div class="settings-row-label">Pasta de dados</div>
              <div class="settings-row-desc">Abre a pasta onde o GitPass salva a configuração.</div>
            </div>
            <div class="settings-row-control">
              <button type="button" class="btn" id="settings-open-data">${icon("folder", 14)}Abrir pasta de dados</button>
            </div>
          </div>
          <div class="settings-row settings-row-stack">
            <div class="settings-keychain">
              <span class="settings-keychain-icon" aria-hidden="true">${icon("key", 18)}</span>
              <div class="settings-keychain-body">
                <div class="settings-row-label">Chaves do macOS (Keychain)</div>
                <p class="settings-note small muted" id="settings-keychain-text">Carregando…</p>
              </div>
            </div>
          </div>
        </div>
      </section>

      <section aria-labelledby="settings-h-about">
        <h2 class="section-label settings-group-label" id="settings-h-about">Sobre</h2>
        <div class="panel settings-group">
          <div class="settings-row settings-about">
            <span class="settings-about-mark" aria-hidden="true">${brandSymbol()}</span>
            <div class="settings-row-text">
              <div class="settings-row-label">GitPass 0.1.0</div>
              <div class="settings-row-desc">A conta certa em cada projeto.</div>
            </div>
          </div>
        </div>
      </section>
    </div>
  `;

  const $ = <T extends HTMLElement>(id: string) => root.querySelector<T>(`#${id}`)!;

  // ----- Aparência -----
  const syncTheme = () => {
    const cur = storedTheme();
    for (const b of root.querySelectorAll<HTMLButtonElement>("[data-theme-mode]")) {
      b.setAttribute("aria-pressed", String(b.dataset.themeMode === cur));
    }
    for (const b of root.querySelectorAll<HTMLButtonElement>("[data-theme-pick]")) {
      b.setAttribute("aria-pressed", String(b.dataset.themePick === cur));
    }
  };
  syncTheme();
  document.addEventListener(THEME_EVENT, syncTheme);

  root.addEventListener("click", (ev) => {
    const target = ev.target as HTMLElement;

    const modeBtn = target.closest<HTMLButtonElement>("[data-theme-mode]");
    if (modeBtn) {
      setTheme(modeBtn.dataset.themeMode as ThemeMode);
      return;
    }
    const pickBtn = target.closest<HTMLButtonElement>("[data-theme-pick]");
    if (pickBtn) {
      setTheme(pickBtn.dataset.themePick as ThemeMode);
      return;
    }

    const copyBtn = target.closest<HTMLButtonElement>("[data-copy]");
    if (copyBtn) {
      void (async () => {
        try {
          await navigator.clipboard.writeText(copyBtn.dataset.copy ?? "");
        } catch (e) {
          showToast(`Não foi possível copiar: ${String(e)}`);
          return;
        }
        copyBtn.innerHTML = icon("check", 16);
        copyBtn.classList.add("settings-copied");
        window.setTimeout(() => {
          if (!copyBtn.isConnected) return;
          copyBtn.innerHTML = icon("copy", 16);
          copyBtn.classList.remove("settings-copied");
        }, 2000);
      })();
    }
  });

  // ----- GitHub -----
  const renderOauth = (st: OauthConfigStatus | null) => {
    const ready = st?.ok === true;
    $("settings-oauth-badge").innerHTML = ready
      ? `<span class="badge success">${icon("check", 12)}Pronta</span>`
      : `<span class="badge error">${icon("x", 12)}Indisponível</span>`;
    $("settings-oauth-desc").textContent = ready
      ? "Conexão com o GitHub pronta."
      : "Este app não tem o GitHub configurado.";
  };
  api
    .oauthStatus()
    .then((st) => alive && renderOauth(st))
    .catch(() => alive && renderOauth(null));

  // ----- Integração com o Git -----
  const intPanel = $("settings-integration");
  let intStatus: IntegrationStatus | null = null;
  let intError: string | null = null;
  let intBusy = false;

  const renderIntegration = () => {
    if (!alive) return;
    intPanel.setAttribute("aria-busy", String(intBusy));
    if (!intStatus) {
      intPanel.innerHTML = `
        <div class="settings-row">
          <div class="settings-row-text">
            <div class="settings-row-label">Não foi possível verificar a integração</div>
            <div class="settings-row-desc">${esc(intError ?? "Algo deu errado.")}</div>
          </div>
          <button type="button" class="btn" data-int="refresh">${icon("refresh", 14)}Tentar de novo</button>
        </div>`;
      return;
    }
    intPanel.innerHTML = renderIntegrationBody(intStatus, intBusy);
  };

  const loadIntegration = async () => {
    try {
      intStatus = await integrationApi.status();
      intError = null;
    } catch (e) {
      intError = errMessage(e);
    }
    renderIntegration();
  };

  const reportResult = (r: IntegrationReport, okMessage: string) => {
    if (r.warnings.length) showToast(r.warnings.join("\n"));
    else showToast(r.changes.length ? `${okMessage}\n${r.changes.join("\n")}` : "Tudo certo. Nada para mudar.", "ok");
  };

  const runIntegration = async (btn: HTMLButtonElement, busyLabel: string, task: () => Promise<void>) => {
    if (intBusy) return;
    intBusy = true;
    for (const b of intPanel.querySelectorAll<HTMLButtonElement>("[data-int]")) b.disabled = true;
    try {
      await withBusy(btn, busyLabel, task);
    } catch (e) {
      showToast(errMessage(e));
    } finally {
      intBusy = false;
      await loadIntegration();
    }
  };

  const onIntegrationClick = (ev: MouseEvent) => {
    const btn = (ev.target as HTMLElement).closest<HTMLButtonElement>("[data-int]");
    if (!btn || btn.disabled) return;
    const action = btn.dataset.int;
    if (action === "refresh") {
      void withBusy(btn, "Verificando…", loadIntegration);
    } else if (action === "repair") {
      void runIntegration(btn, "Reparando…", async () => {
        reportResult(await integrationApi.apply(false), "Integração reparada.");
      });
    } else if (action === "chain-hooks") {
      void (async () => {
        const path = intStatus?.hooks.current_value ?? "";
        const ok = await confirmDialog(btn, {
          title: "Manter também as suas verificações?",
          body: `O Git vai usar as verificações do GitPass, que conferem a conta e depois rodam as suas verificações em ${path}. Para voltar à configuração anterior, use Desfazer integração.`,
          confirm: "Manter as duas",
        });
        if (ok) {
          await runIntegration(btn, "Aplicando…", async () => {
            reportResult(await integrationApi.apply(true), "Verificações combinadas.");
          });
        }
      })();
    } else if (action === "daemon-restart") {
      void runIntegration(btn, "Reiniciando…", async () => {
        await integrationApi.daemonRestart();
        showToast("Serviço em segundo plano reiniciado.", "ok");
      });
    } else if (action === "undo") {
      void (async () => {
        const ok = await confirmDialog(btn, {
          title: "Desfazer a integração com o Git?",
          body: "O GitPass deixa de fornecer as credenciais do GitHub e de validar commits e pushes. A configuração do Git que você tinha antes é restaurada e o serviço em segundo plano é removido. Suas contas e vínculos continuam salvos, e você pode ativar de novo quando quiser.",
          confirm: "Desfazer integração",
          danger: true,
        });
        if (ok) {
          await runIntegration(btn, "Desfazendo…", async () => {
            reportResult(await integrationApi.undo(), "Integração desfeita.");
          });
        }
      })();
    }
  };
  intPanel.addEventListener("click", onIntegrationClick);

  let unlistenIntegration: (() => void) | null = null;
  void onIntegrationChanged((st) => {
    if (intBusy) return;
    intStatus = st;
    intError = null;
    renderIntegration();
  }).then((un) => {
    if (alive) unlistenIntegration = un;
    else un();
  });
  void loadIntegration();

  // ----- Dados e segurança -----
  api
    .configPath()
    .then((p) => alive && ($("settings-config-path").innerHTML = pathBox(p, "caminho")))
    .catch((e) => alive && ($("settings-config-path").innerHTML = `<span class="err small">${esc(String(e))}</span>`));

  const openBtn = $<HTMLButtonElement>("settings-open-data");
  openBtn.addEventListener("click", () => {
    void withBusy(openBtn, "Abrindo…", async () => {
      try {
        await api.openDataFolder();
      } catch (e) {
        showToast(String(e));
      }
    });
  });

  api
    .keychainHint()
    .then((t) => alive && ($("settings-keychain-text").textContent = t))
    .catch((e) => alive && ($("settings-keychain-text").textContent = String(e)));

  return () => {
    alive = false;
    document.removeEventListener(THEME_EVENT, syncTheme);
    intPanel.removeEventListener("click", onIntegrationClick);
    unlistenIntegration?.();
  };
};

// ----- Integração: renderização -----

function checkBadge(state: CheckState): string {
  const b = CHECK_BADGES[state];
  return `<span class="badge ${b.cls}">${icon(b.sym, 12)}${b.label}</span>`;
}

function checkValue(label: string, value: string | null): string {
  if (!value) return "";
  return `<div class="settings-check-value small"><span class="muted">${esc(label)}:</span> <span class="mono">${esc(value)}</span></div>`;
}

function checkRow(id: CheckId, c: ComponentStatus, s: IntegrationStatus, busy: boolean): string {
  const def = CHECKS.find((x) => x.id === id)!;
  const dis = busy ? " disabled" : "";
  let action = "";
  if (id === "hooks" && c.state === "conflict") {
    action = `<button type="button" class="btn" data-int="chain-hooks"${dis}>Manter as duas</button>`;
  } else if (id === "daemon" && s.enabled && c.state !== "missing" && s.binaries.state !== "error") {
    action = `<button type="button" class="btn ghost" data-int="daemon-restart"${dis}>${icon("refresh", 14)}Reiniciar</button>`;
  }
  return `
    <div class="settings-row settings-check" role="group" aria-label="${esc(def.label)}">
      <div class="settings-row-text">
        <div class="settings-row-label">${esc(def.label)}</div>
        <div class="settings-row-desc">${esc(c.summary)}</div>
        ${checkValue(def.current, c.current_value)}
        ${checkValue(def.previous, c.previous_value)}
      </div>
      <div class="settings-row-control settings-row-control-inline">${action}${checkBadge(c.state)}</div>
    </div>`;
}

function renderIntegrationBody(s: IntegrationStatus, busy: boolean): string {
  const dis = busy ? " disabled" : "";
  const callouts: string[] = [];
  if (!s.enabled) {
    callouts.push(
      `<div class="callout warning">${icon("alert", 16)}<span>A integração está desativada: o Git funciona como antes de você instalar o GitPass. Ative para voltar a usar a conta certa em cada repositório.</span></div>`
    );
  }
  if (!s.git_found) {
    callouts.push(`<div class="callout error">${icon("x", 16)}<span>${esc(s.credentials.summary)}</span></div>`);
  } else if (s.binaries.state === "error") {
    callouts.push(`<div class="callout error">${icon("x", 16)}<span>${esc(s.binaries.summary)}</span></div>`);
  }
  const intro = s.ready
    ? "O Git usa automaticamente a conta vinculada a cada repositório."
    : "O GitPass configura o Git sozinho ao abrir. Se algo mudar, clique em Reparar.";
  return `
    ${callouts.length ? `<div class="settings-row settings-row-stack">${callouts.join("")}</div>` : ""}
    ${checkRow("credentials", s.credentials, s, busy)}
    ${checkRow("hooks", s.hooks, s, busy)}
    ${checkRow("daemon", s.daemon, s, busy)}
    <div class="settings-row settings-int-actions">
      <div class="settings-row-desc">${esc(intro)}</div>
      <div class="settings-row-control settings-row-control-inline">
        ${s.enabled ? `<button type="button" class="btn ghost settings-int-undo" data-int="undo"${dis}>Desfazer integração</button>` : ""}
        <button type="button" class="btn${s.ready && s.enabled ? "" : " primary"}" data-int="repair"${dis}>${icon("refresh", 14)}${s.enabled ? "Reparar" : "Ativar integração"}</button>
      </div>
    </div>`;
}

// ----- Diálogo de confirmação -----

type ConfirmOptions = { title: string; body: string; confirm: string; danger?: boolean };

function confirmDialog(opener: HTMLElement, o: ConfirmOptions): Promise<boolean> {
  return new Promise((resolve) => {
    const backdrop = document.createElement("div");
    backdrop.className = "modal-backdrop";
    backdrop.innerHTML = `
      <div class="modal settings-dialog" role="alertdialog" aria-modal="true" aria-labelledby="settings-dialog-title" aria-describedby="settings-dialog-body">
        <h2 class="settings-dialog-title" id="settings-dialog-title">${esc(o.title)}</h2>
        <p class="settings-dialog-body" id="settings-dialog-body">${esc(o.body)}</p>
        <div class="settings-dialog-actions">
          <button type="button" class="btn" data-dialog="cancel">Cancelar</button>
          <button type="button" class="btn ${o.danger ? "settings-btn-danger" : "primary"}" data-dialog="confirm">${esc(o.confirm)}</button>
        </div>
      </div>`;
    const buttons = [...backdrop.querySelectorAll<HTMLButtonElement>("[data-dialog]")];
    const close = (result: boolean) => {
      backdrop.remove();
      if (opener.isConnected) opener.focus();
      resolve(result);
    };
    backdrop.addEventListener("click", (ev) => {
      const t = ev.target as HTMLElement;
      if (t === backdrop) return close(false);
      const b = t.closest<HTMLButtonElement>("[data-dialog]");
      if (b) close(b.dataset.dialog === "confirm");
    });
    backdrop.addEventListener("keydown", (ev) => {
      if (ev.key === "Escape") {
        ev.preventDefault();
        close(false);
      } else if (ev.key === "Tab") {
        ev.preventDefault();
        const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
        buttons[(i + (ev.shiftKey ? buttons.length - 1 : 1)) % buttons.length].focus();
      }
    });
    document.body.appendChild(backdrop);
    buttons[0].focus();
  });
}

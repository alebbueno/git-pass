import "./settings.css";
import { api, type OauthConfigStatus } from "../lib/api";
import { esc, showToast, withBusy } from "../lib/dom";
import { brandSymbol, icon } from "../lib/icons";
import { setTheme, storedTheme, type ThemeMode } from "../lib/theme";
import type { MountView } from "../lib/view";

const START_CMD = "cargo run -p gitpassd --bin gitpassd";
const THEME_EVENT = "gitpass:theme-changed";

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
        <h1 class="view-title">Definições</h1>
        <p class="view-sub">Aparência, ligações e dados locais do GitPass.</p>
      </div>
    </header>

    <div class="settings-page">
      <section aria-labelledby="settings-h-appearance">
        <h2 class="section-label settings-group-label" id="settings-h-appearance">Aparência</h2>
        <div class="panel settings-group">
          <div class="settings-row">
            <div class="settings-row-text">
              <div class="settings-row-label">Tema</div>
              <div class="settings-row-desc">Auto acompanha a definição do sistema.</div>
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
        <h2 class="section-label settings-group-label" id="settings-h-oauth">GitHub OAuth</h2>
        <div class="panel settings-group">
          <div class="settings-row">
            <div class="settings-row-text">
              <div class="settings-row-label">Estado</div>
              <div class="settings-row-desc">Necessário para ligar contas GitHub.</div>
            </div>
            <div class="settings-row-control" id="settings-oauth-badge"><span class="spinner" role="status" aria-label="A carregar"></span></div>
          </div>
          <div class="settings-row settings-row-stack" id="settings-oauth-message" hidden></div>
          <div class="settings-row settings-row-stack">
            <div class="settings-row-label">Ficheiro oauth.env</div>
            <div id="settings-oauth-path"><span class="muted small">A carregar…</span></div>
          </div>
          <div class="settings-row settings-row-stack">
            <p class="settings-note small muted">
              Só é preciso um <strong>client_id</strong> público. O GitPass usa o Device Flow do GitHub, por isso não há <code>client_secret</code>.
              Defina-o na compilação com <code>GITPASS_GITHUB_OAUTH_CLIENT_ID</code> ou no ficheiro <code>oauth.env</code> indicado acima.
            </p>
          </div>
        </div>
      </section>

      <section aria-labelledby="settings-h-daemon">
        <h2 class="section-label settings-group-label" id="settings-h-daemon">Daemon gitpassd</h2>
        <div class="panel settings-group">
          <div class="settings-row">
            <div class="settings-row-text">
              <div class="settings-row-label">Estado</div>
              <div class="settings-row-desc" id="settings-daemon-detail">A testar…</div>
            </div>
            <div class="settings-row-control settings-row-control-inline">
              <span id="settings-daemon-badge"><span class="spinner" role="status" aria-label="A testar"></span></span>
              <button type="button" class="btn" id="settings-daemon-retest">${icon("refresh", 14)}Testar novamente</button>
            </div>
          </div>
          <div class="settings-row settings-row-stack" id="settings-daemon-start" hidden>
            <div class="settings-row-desc">Só é necessário para os hooks Git validarem via daemon. Para o iniciar:</div>
            <div class="settings-code">
              <pre class="settings-code-block mono small"><code>${esc(START_CMD)}</code></pre>
              ${copyButton(START_CMD, "comando")}
            </div>
          </div>
        </div>
      </section>

      <section aria-labelledby="settings-h-data">
        <h2 class="section-label settings-group-label" id="settings-h-data">Dados e segurança</h2>
        <div class="panel settings-group">
          <div class="settings-row settings-row-stack">
            <div class="settings-row-label">Ficheiro de configuração</div>
            <div id="settings-config-path"><span class="muted small">A carregar…</span></div>
          </div>
          <div class="settings-row">
            <div class="settings-row-text">
              <div class="settings-row-label">Pasta de dados</div>
              <div class="settings-row-desc">Abre a pasta onde o GitPass guarda a configuração.</div>
            </div>
            <div class="settings-row-control">
              <button type="button" class="btn" id="settings-open-data">${icon("folder", 14)}Abrir pasta de dados</button>
            </div>
          </div>
          <div class="settings-row settings-row-stack">
            <div class="settings-keychain">
              <span class="settings-keychain-icon" aria-hidden="true">${icon("key", 18)}</span>
              <div class="settings-keychain-body">
                <div class="settings-row-label">Keychain</div>
                <p class="settings-note small muted" id="settings-keychain-text">A carregar…</p>
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

  // ----- GitHub OAuth -----
  const renderOauth = (st: OauthConfigStatus) => {
    $("settings-oauth-badge").innerHTML = st.ok
      ? `<span class="badge success">${icon("check", 12)}Configurado</span>`
      : `<span class="badge error">${icon("alert", 12)}Em falta</span>`;
    const msg = $("settings-oauth-message");
    msg.hidden = !st.message;
    msg.innerHTML = st.message ? `<p class="settings-message small">${esc(st.message)}</p>` : "";
    $("settings-oauth-path").innerHTML = pathBox(st.oauth_env_path, "caminho");
  };
  api
    .oauthStatus()
    .then((st) => alive && renderOauth(st))
    .catch((e) => {
      if (!alive) return;
      $("settings-oauth-badge").innerHTML = `<span class="badge error">${icon("alert", 12)}Em falta</span>`;
      const msg = $("settings-oauth-message");
      msg.hidden = false;
      msg.innerHTML = `<p class="settings-message small">${esc(String(e))}</p>`;
      $("settings-oauth-path").innerHTML = `<span class="muted small">Indisponível.</span>`;
    });

  // ----- Daemon -----
  const retest = $<HTMLButtonElement>("settings-daemon-retest");
  const testDaemon = async () => {
    try {
      const msg = await api.pingDaemon();
      if (!alive) return;
      $("settings-daemon-badge").innerHTML = `<span class="badge success">${icon("check", 12)}Ativo</span>`;
      $("settings-daemon-detail").textContent = msg;
      $("settings-daemon-start").hidden = true;
    } catch {
      if (!alive) return;
      $("settings-daemon-badge").innerHTML = `<span class="badge warning">${icon("alert", 12)}Parado</span>`;
      $("settings-daemon-detail").textContent = "Só é necessário para os hooks Git validarem via daemon.";
      $("settings-daemon-start").hidden = false;
    }
  };
  void testDaemon();
  retest.addEventListener("click", () => {
    void withBusy(retest, "A testar…", testDaemon);
  });

  // ----- Dados e segurança -----
  api
    .configPath()
    .then((p) => alive && ($("settings-config-path").innerHTML = pathBox(p, "caminho")))
    .catch((e) => alive && ($("settings-config-path").innerHTML = `<span class="err small">${esc(String(e))}</span>`));

  const openBtn = $<HTMLButtonElement>("settings-open-data");
  openBtn.addEventListener("click", () => {
    void withBusy(openBtn, "A abrir…", async () => {
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
  };
};

import "./credits.css";
import { api } from "../lib/api";
import { errMessage, esc, fullDateTime, showToast, withBusy } from "../lib/dom";
import { brandSymbol, icon } from "../lib/icons";
import { formatSupportReport, SUPPORT, type SupportReportInput } from "../lib/support-info";
import { systemApi, type Diagnostics } from "../lib/api-system";
import type { MountView, ViewId } from "../lib/view";

function linkRow(title: string, desc: string, url: string): string {
  return `<li>
    <button type="button" class="credits-link" data-open-url="${esc(url)}">
      <span><strong>${esc(title)}</strong><span class="caption muted">${esc(desc)}</span></span>
      ${icon("external")}
    </button>
  </li>`;
}

function metaRow(label: string, value: string | null, loading = false): string {
  const dd = loading ? `<span class="skeleton" style="width:8rem;height:1em;display:inline-block"></span>` : value ? esc(value) : `<span class="muted">—</span>`;
  return `<div class="credits-meta-row"><dt>${esc(label)}</dt><dd>${dd}</dd></div>`;
}

export const mount: MountView = (root, ctx) => {
  let alive = true;
  let report: SupportReportInput | null = null;

  root.innerHTML = `
    <div class="page-head">
      <div class="heading-group">
        <h1>Créditos e suporte</h1>
        <p>Versão do app, como pedir ajuda e reconhecimentos.</p>
      </div>
      <div class="page-actions">
        <button type="button" class="control" data-action="copy-report" disabled>${icon("copy")}<span>Copiar dados para suporte</span></button>
        <button type="button" class="control primary" data-action="export-logs">${icon("download")}<span>Exportar logs…</span></button>
      </div>
    </div>

    <div class="credits">
      <header class="credits-hero" aria-labelledby="credits-product-title">
        <div class="credits-hero-mark" aria-hidden="true">${brandSymbol()}</div>
        <div class="credits-hero-copy">
          <p class="eyebrow">GitPass</p>
          <h2 id="credits-product-title">${esc(SUPPORT.productName)}</h2>
          <p class="caption muted">A conta certa em cada projeto — identidades Git e contas GitHub no macOS.</p>
        </div>
      </header>

      <section class="surface" aria-labelledby="credits-h-support">
        <h2 class="section-title" id="credits-h-support">Suporte</h2>
        <p class="caption muted">Antes de abrir um chamado, rode o Diagnóstico e exporte os logs. O pacote não inclui senhas nem tokens.</p>
        <ol class="credits-steps">
          <li>Abra <strong>Diagnóstico</strong> e corrija itens em erro, se houver.</li>
          <li>Use <strong>Exportar logs</strong> (aqui ou no Diagnóstico) e guarde o arquivo ZIP.</li>
          <li>Copie os <strong>dados para suporte</strong> abaixo e cole na issue ou mensagem.</li>
          <li>Descreva o que fazia, o repositório (sem segredos) e se viu algum código <span class="mono">GP-*</span>.</li>
        </ol>
        <div class="row-actions" style="margin-top: var(--sp-3)">
          <button type="button" class="control" data-go="diagnostics">${icon("stethoscope")}Abrir Diagnóstico</button>
          <button type="button" class="control ghost" data-go="integration">${icon("plug")}Integração</button>
        </div>
        <div class="credits-report" id="credits-report-box" hidden>
          <strong>Pré-visualização</strong>
          <pre id="credits-report-text"></pre>
        </div>
      </section>

      <section class="surface" aria-labelledby="credits-h-meta">
        <h2 class="section-title" id="credits-h-meta">Este dispositivo</h2>
        <dl class="credits-meta" id="credits-meta" aria-busy="true">
          ${metaRow("Versão do app", null, true)}
          ${metaRow("macOS", null, true)}
          ${metaRow("Git", null, true)}
          ${metaRow("Arquivo de configuração", null, true)}
          ${metaRow("Bundle", SUPPORT.bundleId)}
          ${metaRow("Keychain", SUPPORT.keychainService)}
        </dl>
      </section>

      <section class="surface" aria-labelledby="credits-h-links">
        <h2 class="section-title" id="credits-h-links">Links</h2>
        <ul class="credits-links">
          ${linkRow("Relatar problema", "Abrir issues no GitHub", SUPPORT.issuesUrl)}
          ${linkRow("Código-fonte", "Repositório do projeto", SUPPORT.repoUrl)}
          ${linkRow("Especificação do produto", "Documento de design v1", SUPPORT.designDocUrl)}
        </ul>
      </section>

      <section class="surface" aria-labelledby="credits-h-oss">
        <h2 class="section-title" id="credits-h-oss">Créditos</h2>
        <ul class="credits-oss">
          <li><strong>GitPass</strong><span>Desenvolvido pelos contribuidores do projeto (Rust + Tauri).</span></li>
          <li><strong>Interface</strong><span>TypeScript, Vite e Web APIs no shell desktop.</span></li>
          <li><strong>Ícones</strong><span>Família Rota — desenho próprio do GitPass.</span></li>
          <li><strong>GitHub</strong><span>OAuth, API e avatares conforme termos do GitHub.</span></li>
        </ul>
        <p class="caption muted" style="margin-top: var(--sp-3)">
          Tokens OAuth ficam no Keychain do macOS; o <code class="mono">config.toml</code> guarda só dados não sensíveis.
        </p>
      </section>
    </div>
  `;

  const meta = root.querySelector<HTMLElement>("#credits-meta")!;
  const copyBtn = root.querySelector<HTMLButtonElement>('[data-action="copy-report"]')!;
  const reportBox = root.querySelector<HTMLElement>("#credits-report-box")!;
  const reportPre = root.querySelector<HTMLElement>("#credits-report-text")!;

  const paintMeta = (d: Diagnostics, configPath: string): void => {
    meta.setAttribute("aria-busy", "false");
    meta.innerHTML = `
      ${metaRow("Versão do app", d.app_version)}
      ${metaRow("macOS", d.macos_version)}
      ${metaRow("Git", d.git_version)}
      ${metaRow("Arquivo de configuração", configPath)}
      ${metaRow("Bundle", SUPPORT.bundleId)}
      ${metaRow("Keychain", SUPPORT.keychainService)}
    `;
    report = {
      appVersion: d.app_version,
      macosVersion: d.macos_version,
      gitVersion: d.git_version,
      configPath,
      generatedAt: fullDateTime(d.generated_at),
    };
    const text = formatSupportReport(report);
    reportPre.textContent = text;
    reportBox.hidden = false;
    copyBtn.disabled = false;
  };

  void Promise.all([systemApi.diagnostics({ probeTokens: false }), api.configPath()])
    .then(([d, configPath]) => {
      if (!alive) return;
      paintMeta(d, configPath);
    })
    .catch((e) => {
      if (!alive) return;
      meta.setAttribute("aria-busy", "false");
      meta.innerHTML = `<p class="caption err">${esc(errMessage(e))}</p>`;
      showToast(errMessage(e), "err");
    });

  root.addEventListener("click", (ev) => {
    const target = ev.target as HTMLElement;
    const urlBtn = target.closest<HTMLElement>("[data-open-url]");
    if (urlBtn?.dataset.openUrl) {
      api.openExternal(urlBtn.dataset.openUrl).catch((e) => showToast(errMessage(e), "err"));
      return;
    }
    const go = target.closest<HTMLElement>("[data-go]");
    if (go?.dataset.go) {
      ctx.navigate(go.dataset.go as ViewId);
      return;
    }
    const act = target.closest<HTMLButtonElement>("[data-action]");
    if (!act) return;
    switch (act.dataset.action) {
      case "copy-report":
        if (!report) return;
        void navigator.clipboard.writeText(formatSupportReport(report)).then(
          () => showToast("Dados copiados.", "ok"),
          () => showToast("Não foi possível copiar.", "err")
        );
        break;
      case "export-logs":
        void withBusy(act, "Exportando…", async () => {
          const path = await systemApi.exportLogs();
          if (!path) return;
          showToast("Logs exportados.", "ok");
          await navigator.clipboard.writeText(path).catch(() => undefined);
        }).catch((e) => showToast(errMessage(e), "err"));
        break;
    }
  });

  return () => {
    alive = false;
  };
};

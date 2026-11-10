/** URLs e identificadores públicos para suporte (sem segredos). */
export const SUPPORT = {
  productName: "GitPass",
  bundleId: "com.gitpass.GitPass",
  keychainService: "com.gitpass.GitPass",
  repoUrl: "https://github.com/alebbueno/git-pass",
  issuesUrl: "https://github.com/alebbueno/git-pass/issues",
  designDocUrl: "https://github.com/alebbueno/git-pass/blob/main/docs/gitpass-v1-design.md",
  buildGateDocUrl: "https://github.com/alebbueno/git-pass/blob/main/docs/agents/build-gate.md",
} as const;

export type SupportReportInput = {
  appVersion: string;
  macosVersion: string | null;
  gitVersion: string | null;
  configPath: string | null;
  generatedAt: string;
};

/** Texto para colar em e-mail, issue ou chat de suporte (sem tokens). */
export function formatSupportReport(input: SupportReportInput): string {
  const lines = [
    "—— GitPass · dados para suporte ——",
    `Produto: ${SUPPORT.productName}`,
    `Versão: ${input.appVersion}`,
    `Bundle: ${SUPPORT.bundleId}`,
    `macOS: ${input.macosVersion ?? "desconhecido"}`,
    `Git: ${input.gitVersion ?? "desconhecido"}`,
    `Config: ${input.configPath ?? "não disponível"}`,
    `Gerado: ${input.generatedAt}`,
    `Repositório: ${SUPPORT.repoUrl}`,
    "",
    "Anexe também o ZIP de logs (Diagnóstico → Exportar logs), se possível.",
  ];
  return lines.join("\n");
}

# Contribuir

## `gitpass-desktop` (UI web / Tauri)

Após qualquer alteração em `gitpass-desktop/src/`, `gitpass-desktop/index.html` ou assets de branding, **gerar sempre a build de produção** para que `gitpass-desktop/dist/` fique alinhado com o código:

```bash
cd gitpass-desktop && npm run build
```

Para testar o shell Tauri com o bundle atual: `npm run tauri dev` ou `npm run tauri build` para o `.app`.

### Verificação automática (build-gate)

Depois de mudanças no app, rode a suíte completa (TypeScript, Vite, contrato Rust, `GitPass.app`):

```bash
./scripts/verify-app-build.sh
```

Modo rápido sem Tauri: `./scripts/verify-app-build.sh --quick`

**Agente em worktree** (mesmo fluxo dos outros agentes): [docs/agents/build-gate.md](docs/agents/build-gate.md). Disparo:

```bash
./scripts/run-build-gate-agent.sh
```

O relatório sai em `/tmp/gitpass-agents/build-gate-report.md`. No GitHub, o workflow `.github/workflows/build-gate.yml` roda nos pushes/PRs que mexem no desktop.

Ver também: [docs/GitPass_Desktop_UI_RepoID.md](docs/GitPass_Desktop_UI_RepoID.md).

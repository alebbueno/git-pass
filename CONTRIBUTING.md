# Contribuir

## `gitpass-desktop` (UI web / Tauri)

Após qualquer alteração em `gitpass-desktop/src/`, `gitpass-desktop/index.html` ou assets de branding, **gerar sempre a build de produção** para que `gitpass-desktop/dist/` fique alinhado com o código:

```bash
cd gitpass-desktop && npm run build
```

Para testar o shell Tauri com o bundle atual: `npm run tauri dev` ou `npm run tauri build` para o `.app`.

Ver também: [docs/GitPass_Desktop_UI_RepoID.md](docs/GitPass_Desktop_UI_RepoID.md).

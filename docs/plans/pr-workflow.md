# Pull requests — fluxo completo (GitHub)

## Status

| Fase | Escopo | Status |
|------|--------|--------|
| **1** | API: criar, detalhar, revisar (approve / request changes), merge | Em código |
| **2** | UI: painel na aba «Pull requests», criar PR manual | Em código |
| **3** | Criar PR após push + lista por repo + prévia de diff | Em código |
| **4** | Bitbucket PR (inbox, diff, listar, criar, merge) | Em código |
| **5** | Checks CI no painel, merge squash/rebase, comentário por arquivo | Em código |

## Fase 1 — Backend (`github_pr.rs`)

- `POST /repos/{owner}/{repo}/pulls` — criar
- `GET /repos/{owner}/{repo}/pulls/{n}` — detalhe + permissões
- `GET .../pulls/{n}/reviews` — histórico
- `POST .../pulls/{n}/reviews` — `APPROVE` | `REQUEST_CHANGES` | `COMMENT`
- `PUT .../pulls/{n}/merge` — merge (merge / squash / rebase)

`can_merge`: admin ou maintain, PR aberto e `mergeable`.  
`can_review`: não autor, token com push/triage/admin/maintain.

## Fase 2 — UI

- **Na nuvem → Pull requests**: lista (inbox existente) abre **painel** com ações.
- **Novo PR**: formulário owner/repo/head/base.
- Atalho **Abrir no GitHub** para diff completo.

## Fase 3 — Repositório vinculado

- Aba **Pull requests** na gaveta do projeto (lista abertos + «Abrir PR deste branch»).
- Painel: lista de arquivos com patch unified (até 25 arquivos; restante no GitHub).
- Após **Enviar**: botão **Abrir pull request** quando head ≠ default.

## Próximos passos

1. Auto-merge quando checks passarem.
2. Bitbucket Cloud (pull requests).
3. Comentários em linhas deletadas (lado LEFT).

**Bitbucket inbox:** cache 45s, repos vinculados primeiro, filtro `reviewers.nickname` por repo (menos chamadas).

**Bitbucket painel:** aprovar PR (`POST .../approve`) quando você é revisor.
4. Bitbucket Cloud REST (pullrequests).

## Escopos OAuth

O fluxo GitHub atual já pede `repo` (e org). Revisão e merge exigem permissão de escrita no repositório.

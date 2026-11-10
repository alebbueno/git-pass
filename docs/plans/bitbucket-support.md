# Bitbucket (Cloud) — plano e status

> **Status:** Fase 0 em andamento (v0.2). GitHub permanece o provedor principal; Bitbucket entra de forma incremental.

## Objetivo

Permitir **conta Bitbucket Cloud** no GitPass, ao lado do GitHub, para:

- vincular clones com `origin` em `bitbucket.org`;
- guardar **API token** Atlassian (ou OAuth) no Keychain;
- clonar via HTTPS com a mesma política de não gravar credenciais no `origin`.

## Fases

| Fase | Escopo | Status |
|------|--------|--------|
| **0** | `Account.host`, parse de remoto Bitbucket, vincular pasta, API token, lista de contas, descoberta local | Em código |
| **1** | Nuvem: listar/clonar repositórios Bitbucket na UI | Planejado |
| **2** | OAuth consumer embutido (paridade GitHub), criar repositório, Bitbucket Server / Data Center | Futuro |

## Modelo de dados

- `Account.host`: `github` (padrão) | `bitbucket`.
- Login Bitbucket: `Account.github_login` (campo legado; nome do utilizador Atlassian).
- Keychain: item `bitbucket.app_password.{account_id}` (JSON legado; campo `app_password` = API token ou access token OAuth).
- `RepoBinding.github_owner` / `github_repo`: workspace + slug Bitbucket (mesmos campos que no GitHub).

## Autenticação v1

[API tokens Atlassian](https://id.atlassian.com/manage-profile/security/api-tokens) com escopos Bitbucket `read:repository:bitbucket` e `write:repository:bitbucket`. App passwords foram descontinuadas.

**OAuth (opcional):** consumer Bitbucket com callback `http://127.0.0.1:8766/callback` e `BITBUCKET_OAUTH_CLIENT_ID` / `BITBUCKET_OAUTH_CLIENT_SECRET` em `oauth.env` (sem device flow — abre o navegador como no OAuth web do GitHub).

## Não-objetivos da Fase 0

- OAuth Bitbucket na loja Atlassian.
- PR inbox / APIs de integração paridade GitHub.
- Self-hosted Bitbucket.

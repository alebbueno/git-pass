import { invoke } from "@tauri-apps/api/core";

export type PrReviewRow = {
  id: number;
  user_login: string;
  state: string;
  body: string | null;
  submitted_at: string | null;
};

export type RepoPermissions = {
  admin: boolean;
  maintain: boolean;
  push: boolean;
  triage: boolean;
  pull: boolean;
};

export type PullRequestDetail = {
  number: number;
  title: string;
  body: string | null;
  state: string;
  html_url: string;
  draft: boolean;
  merged: boolean;
  mergeable: boolean | null;
  head_ref: string;
  base_ref: string;
  author_login: string;
  additions: number;
  deletions: number;
  changed_files: number;
  viewer_login: string;
  permissions: RepoPermissions;
  reviews: PrReviewRow[];
  can_merge: boolean;
  can_review: boolean;
  repo_full_name: string;
  head_sha: string;
  checks: PrCheckRow[];
  comments: PrReviewComment[];
  requested_reviewers: string[];
  can_request_reviewers: boolean;
  auto_merge_enabled: boolean;
  can_enable_auto_merge: boolean;
};

export type PrCheckRow = {
  name: string;
  status: string;
  conclusion: string | null;
  html_url: string | null;
};

export type PrReviewComment = {
  id: number;
  user_login: string;
  body: string;
  path: string;
  line: number | null;
  created_at: string;
};

export type PullRequestCreated = {
  number: number;
  title: string;
  html_url: string;
  state: string;
  draft: boolean;
  head_ref: string;
  base_ref: string;
};

export type MergePullRequestResult = {
  merged: boolean;
  message: string;
  sha: string | null;
};

export type RepoPullRequestRow = {
  number: number;
  title: string;
  html_url: string;
  state: string;
  draft: boolean;
  head_ref: string;
  base_ref: string;
  author_login: string;
  updated_at: string;
};

export type PrFileChange = {
  filename: string;
  status: string;
  additions: number;
  deletions: number;
  patch: string | null;
};

export const prApi = {
  listRepo: (accountId: string, owner: string, repo: string) =>
    invoke<RepoPullRequestRow[]>("github_pr_list_repo", { accountId, owner, repo }),

  files: (accountId: string, owner: string, repo: string, number: number) =>
    invoke<PrFileChange[]>("github_pr_files", { accountId, owner, repo, number }),

  detail: (accountId: string, owner: string, repo: string, number: number) =>
    invoke<PullRequestDetail>("github_pr_detail", { accountId, owner, repo, number }),

  create: (
    accountId: string,
    owner: string,
    repo: string,
    title: string,
    head: string,
    base: string,
    body?: string | null,
    draft?: boolean
  ) =>
    invoke<PullRequestCreated>("github_pr_create", {
      accountId,
      owner,
      repo,
      title,
      head,
      base,
      body: body ?? null,
      draft: draft ?? false,
    }),

  review: (
    accountId: string,
    owner: string,
    repo: string,
    number: number,
    event: "approve" | "request_changes" | "comment",
    body?: string | null
  ) =>
    invoke<void>("github_pr_submit_review", {
      accountId,
      owner,
      repo,
      number,
      event,
      body: body ?? null,
    }),

  merge: (accountId: string, owner: string, repo: string, number: number, method?: "merge" | "squash" | "rebase") =>
    invoke<MergePullRequestResult>("github_pr_merge", {
      accountId,
      owner,
      repo,
      number,
      method: method ?? "merge",
    }),

  commentFile: (
    accountId: string,
    owner: string,
    repo: string,
    number: number,
    headSha: string,
    path: string,
    body: string,
    line?: number | null,
    side?: "LEFT" | "RIGHT" | null
  ) =>
    invoke<void>("github_pr_comment_file", {
      accountId,
      owner,
      repo,
      number,
      headSha,
      path,
      body,
      line: line ?? null,
      side: side ?? "RIGHT",
    }),

  requestReviewers: (accountId: string, owner: string, repo: string, number: number, reviewers: string[]) =>
    invoke<void>("github_pr_request_reviewers", { accountId, owner, repo, number, reviewers }),

  enableAutoMerge: (
    accountId: string,
    owner: string,
    repo: string,
    number: number,
    method?: "merge" | "squash" | "rebase"
  ) =>
    invoke<void>("github_pr_enable_auto_merge", {
      accountId,
      owner,
      repo,
      number,
      method: method ?? "merge",
    }),

  disableAutoMerge: (accountId: string, owner: string, repo: string, number: number) =>
    invoke<void>("github_pr_disable_auto_merge", { accountId, owner, repo, number }),
};

export const bbPrApi = {
  listRepo: (accountId: string, workspace: string, repoSlug: string) =>
    invoke<RepoPullRequestRow[]>("bitbucket_pr_list_repo", { accountId, workspace, repoSlug }),

  detail: (accountId: string, workspace: string, repoSlug: string, number: number) =>
    invoke<PullRequestDetail>("bitbucket_pr_detail", { accountId, workspace, repoSlug, number }),

  create: (
    accountId: string,
    workspace: string,
    repoSlug: string,
    title: string,
    head: string,
    base: string,
    body?: string | null,
    draft?: boolean
  ) =>
    invoke<PullRequestCreated>("bitbucket_pr_create", {
      accountId,
      workspace,
      repoSlug,
      title,
      head,
      base,
      body: body ?? null,
      draft: draft ?? false,
    }),

  merge: (accountId: string, workspace: string, repoSlug: string, number: number) =>
    invoke<MergePullRequestResult>("bitbucket_pr_merge", { accountId, workspace, repoSlug, number }),

  diff: (accountId: string, workspace: string, repoSlug: string, number: number) =>
    invoke<string>("bitbucket_pr_diff", { accountId, workspace, repoSlug, number }),

  approve: (accountId: string, workspace: string, repoSlug: string, number: number) =>
    invoke<void>("bitbucket_pr_approve", { accountId, workspace, repoSlug, number }),
};

export function splitRepoFullName(full: string): { owner: string; repo: string } | null {
  const i = full.indexOf("/");
  if (i <= 0 || i >= full.length - 1) return null;
  return { owner: full.slice(0, i), repo: full.slice(i + 1) };
}

import type { AccountRow } from "./api";
import { esc } from "./dom";

/** Famílias de cor de perfil do kit RepoID (`--rid-account-*`). Cada conta fica sempre com a mesma. */
export const PROFILE_FAMILIES = ["iris", "aqua", "blue", "rose"] as const;
export type ProfileFamily = (typeof PROFILE_FAMILIES)[number];

export function profileFamily(accountId: string): ProfileFamily {
  let h = 0;
  for (let i = 0; i < accountId.length; i++) {
    h = (h * 31 + accountId.charCodeAt(i)) | 0;
  }
  return PROFILE_FAMILIES[Math.abs(h) % PROFILE_FAMILIES.length];
}

export function profileName(a: Pick<AccountRow, "id" | "login" | "label">): string {
  return a.label || a.login || a.id.slice(0, 8);
}

export function profileInitials(name: string): string {
  const parts = name.replace(/^@/, "").split(/[\s._-]+/).filter(Boolean);
  const letters = parts.length >= 2 ? parts[0][0] + parts[1][0] : name.replace(/^@/, "").slice(0, 2);
  return letters.toUpperCase();
}

/**
 * Chip de perfil do kit: iniciais + nome sobre a cor da família.
 * O nome fica sempre visível (a cor nunca é o único indicador).
 */
export function profileChip(
  a: Pick<AccountRow, "id" | "login" | "label">,
  opts: { size?: "sm" | "md"; showName?: boolean } = {}
): string {
  const name = profileName(a);
  const fam = profileFamily(a.id);
  const size = opts.size ?? "sm";
  const nameHtml = opts.showName === false ? "" : `<span class="profile-chip-name">${esc(name)}</span>`;
  return `<span class="profile-chip profile-${fam} profile-chip-${size}" title="${esc(name)}"><span class="profile-chip-initials" aria-hidden="true">${esc(profileInitials(name))}</span>${nameHtml}</span>`;
}

/** Avatar circular só com iniciais (para listas onde o nome já aparece ao lado). */
export function profileAvatar(a: Pick<AccountRow, "id" | "login" | "label">, size = 36): string {
  const name = profileName(a);
  return `<span class="profile-avatar profile-${profileFamily(a.id)}" style="--avatar-size:${size}px" aria-hidden="true">${esc(profileInitials(name))}</span>`;
}

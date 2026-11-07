import type { AccountRow } from "./api";
import { esc } from "./dom";

/** Famílias de cor de perfil (`--iris`, `--aqua`, `--blue`, `--rose`). Cada conta fica sempre com a mesma. */
export const PROFILE_FAMILIES = ["iris", "aqua", "blue", "rose"] as const;
export type ProfileFamily = (typeof PROFILE_FAMILIES)[number];

type ProfileLike = Pick<AccountRow, "id" | "login" | "label"> & { name?: string | null };

export function profileFamily(accountId: string): ProfileFamily {
  let h = 0;
  for (let i = 0; i < accountId.length; i++) {
    h = (h * 31 + accountId.charCodeAt(i)) | 0;
  }
  return PROFILE_FAMILIES[Math.abs(h) % PROFILE_FAMILIES.length];
}

export function profileName(a: ProfileLike): string {
  return a.label || a.name || a.login || a.id.slice(0, 8);
}

export function profileInitials(name: string): string {
  const plain = name.replace(/^@/, "").normalize("NFD").replace(/\p{Diacritic}/gu, "");
  const parts = plain.split(/[\s._-]+/).filter(Boolean);
  const letters = parts.length >= 2 ? parts[0][0] + parts[1][0] : plain.slice(0, 2);
  return letters.toUpperCase();
}

/** Avatar quadrado com iniciais na cor do perfil. */
export function profileAvatar(a: ProfileLike, size = 31): string {
  const name = profileName(a);
  return `<span class="avatar ${profileFamily(a.id)}" style="--size:${size}px" aria-hidden="true">${esc(profileInitials(name))}</span>`;
}

/**
 * Selo do perfil (`.pill`): iniciais + nome sobre a cor da família.
 * O nome fica sempre visível (a cor nunca é o único indicador).
 */
export function profileChip(a: ProfileLike, opts: { size?: "sm" | "md"; showName?: boolean } = {}): string {
  const name = profileName(a);
  const fam = profileFamily(a.id);
  const nameHtml = opts.showName === false ? "" : `<span class="truncate">${esc(name)}</span>`;
  return `<span class="pill ${fam}${opts.size === "md" ? " pill-md" : ""}" title="${esc(name)}"><span class="avatar" aria-hidden="true">${esc(
    profileInitials(name)
  )}</span>${nameHtml}</span>`;
}

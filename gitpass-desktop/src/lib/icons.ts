/** Ícones GitPass · família Rota (24×24, traço 1.8). */
const paths = {
  logo: '<path d="M5.7 17.5h3.7a3 3 0 0 0 2.2-1l4.8-5.4a2.9 2.9 0 0 1 2.2-1h.4M11.5 16.4v-5.1a3.5 3.5 0 0 0-3.5-3.5H7.2"/><circle cx="4.2" cy="17.5" r="1.8" fill="currentColor" stroke="none" data-node=""/><circle cx="5.8" cy="7.8" r="1.6"/><circle cx="20" cy="10.1" r="1.85" fill="currentColor" stroke="none" data-node=""/>',
  dashboard:
    '<rect x="3.5" y="3.5" width="7" height="7" rx="1.5"/><rect x="13.5" y="3.5" width="7" height="7" rx="1.5"/><rect x="3.5" y="13.5" width="7" height="7" rx="1.5"/><rect x="13.5" y="13.5" width="7" height="7" rx="1.5"/>',
  repo: '<path d="M4.5 18V5.5a2 2 0 0 1 2-2H16l3.5 3.5V20.5H7a2.5 2.5 0 0 1 0-5h12.5M15.5 4v3.5H19M8 11h3"/><circle cx="14.9" cy="11" r="1.35" fill="currentColor" stroke="none" data-node=""/>',
  user: '<circle cx="11" cy="7.2" r="3.4"/><path d="M3.7 20.2v-1.1c0-3.5 3-5.5 7.3-5.5 3.1 0 5.5 1.1 6.6 3.1"/><circle cx="19.8" cy="19.7" r="1.55" fill="currentColor" stroke="none" data-node=""/>',
  cloud: '<path d="M8 18.5H6.7a3.7 3.7 0 0 1-.8-7.3 5.9 5.9 0 0 1 11.1-1.8h.2a4.6 4.6 0 0 1 0 9.1h-3.5"/><circle cx="10.8" cy="18.5" r="1.5" fill="currentColor" stroke="none" data-node=""/>',
  settings: '<path d="M4 6h4m5 0h7M4 12h9m5 0h2M4 18h2m5 0h9"/><circle cx="10.5" cy="6" r="2.2"/><circle cx="15.5" cy="12" r="2.2"/><circle cx="8.5" cy="18" r="2.2"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  refresh: '<path d="M19.6 9.2A8 8 0 1 0 20 14m-.4-9.4v4.6H15"/>',
  folder: '<path d="M3.5 7a2 2 0 0 1 2-2h4l2 2h7a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2ZM3.5 10.5h17"/>',
  external: '<path d="M13.8 4.4h5.8v5.8m0-5.8-8.4 8.4m7.6-.1v4.9a2 2 0 0 1-2 2H6.4a2 2 0 0 1-2-2V7.2a2 2 0 0 1 2-2h4.9"/>',
  copy: '<rect x="8.5" y="8.5" width="11.5" height="11.5" rx="2"/><path d="M15 6.4V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h.4"/>',
  check: '<path d="m5 12.3 4.4 4.4L19.6 6.5"/>',
  alert: '<path d="M10.6 4.8 3.1 18a1.5 1.5 0 0 0 1.3 2.3h15.2a1.5 1.5 0 0 0 1.3-2.3L13.4 4.8a1.6 1.6 0 0 0-2.8 0Z"/><path d="M12 9.3v3.9"/><circle cx="12" cy="16.7" r="1.2" fill="currentColor" stroke="none" data-node=""/>',
  x: '<path d="m6.5 6.5 11 11m-11 0 11-11"/>',
  search: '<circle cx="10.4" cy="10.4" r="5.9"/><path d="m15 15 5.2 5.2"/>',
  lock: '<rect x="5" y="10" width="14" height="10.5" rx="2.1"/><path d="M8.2 10V7.3a3.8 3.8 0 1 1 7.6 0V10"/><circle cx="12" cy="15.3" r="1.2" fill="currentColor" stroke="none" data-node=""/>',
  key: '<circle cx="7.9" cy="15.8" r="3.9"/><path d="m10.6 13.1 9.2-9.2m-2.7 2.7 2.5 2.5m-5.4.3 2 2"/>',
  branch: '<path d="M6 7.4v9.2M6 13.5h3.2a4 4 0 0 0 2.9-1.2l2.5-2.6a4.1 4.1 0 0 1 2.9-1.2h.4"/><circle cx="6" cy="5.5" r="1.9"/><circle cx="6" cy="18.5" r="1.9"/><circle cx="19.2" cy="8.5" r="1.85" fill="currentColor" stroke="none" data-node=""/>',
  commit: '<path d="M3 12h5.7m6.6 0H21"/><circle cx="12" cy="12" r="3.2"/><circle cx="12" cy="12" r="1.1" fill="currentColor" stroke="none" data-node=""/>',
  merge: '<circle cx="17" cy="17" r="2.3"/><circle cx="7" cy="6" r="2.3"/><path d="M7 20V8.3a8.5 8.5 0 0 0 7.7 8.7"/>',
  sun: '<circle cx="12" cy="12" r="4"/><path d="M12 3v2m0 14v2m9-9h-2M5 12H3m15.4-6.4L17 7M7 17l-1.4 1.4m12.8 0L17 17M7 7 5.6 5.6"/>',
  moon: '<path d="M20.2 14.6a8.1 8.1 0 1 1-10.8-10 7 7 0 0 0 10.8 10Z"/>',
  monitor: '<rect x="3.4" y="4.6" width="17.2" height="12.3" rx="2.1"/><path d="M12 17v3m-3.7 0h7.4"/>',
  activity: '<path d="M3.8 13h3.1l2.4-5.2 3.5 8.2 2.4-5h3.3"/><circle cx="20" cy="11" r="1.65" fill="currentColor" stroke="none" data-node=""/>',
  arrowUp: '<path d="M12 19V5.5M6 11.5l6-6 6 6"/>',
  arrowDown: '<path d="M12 5v13.5M18 12.5l-6 6-6-6"/>',
  back: '<path d="M19.5 12H4.5m5.2-5.2L4.5 12l5.2 5.2"/>',
  chevronLeft: '<path d="m15 6-6 6 6 6"/>',
  chevronRight: '<path d="m9.2 6.5 5.6 5.5-5.6 5.5"/>',
  chevronDown: '<path d="m6.5 9.2 5.5 5.6 5.5-5.6"/>',
  more: '<circle cx="5.2" cy="12" r="1.45" fill="currentColor" stroke="none" data-node=""/><circle cx="12" cy="12" r="1.45" fill="currentColor" stroke="none" data-node=""/><circle cx="18.8" cy="12" r="1.45" fill="currentColor" stroke="none" data-node=""/>',
  file: '<path d="M13.5 3.5h-6a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h9a2 2 0 0 0 2-2V8.5Z"/><path d="M13.5 3.5v5h5"/>',
  info: '<circle cx="12" cy="12" r="8.4"/><path d="M12 11.4v4.8"/><circle cx="12" cy="8" r="1.17" fill="currentColor" stroke="none" data-node=""/>',
  edit: '<path d="M12.5 19.5h7"/><path d="M15.8 4.7a2 2 0 0 1 2.9 2.9L8.2 18.1l-3.7.9.9-3.7Z"/>',
  terminal: '<rect x="3.5" y="5.2" width="17" height="13.6" rx="2.4"/><path d="m7.2 9.3 2.8 2.7-2.8 2.7m5.5 0h4"/>',
  send: '<path d="M12 15V4.4m-4.1 4L12 4.3l4.1 4.1M4.5 13.5v5a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2v-5"/>',
  pin: '<path d="M12 16.5V21"/><path d="M9.2 3.5h5.6l-.9 5.2 2.9 2.9v2H7.2v-2l2.9-2.9Z"/>',
  pinOff: '<path d="M12 16.5V21"/><path d="M14.8 3.5H9.2l.5 3M13.9 8.7l2.9 2.9v2h-3M10.2 13.6H7.2v-2l2.2-2.2"/><path d="m4 4 16 16"/>',
  target: '<circle cx="12" cy="12" r="8.5"/><circle cx="12" cy="12" r="4.5"/><circle cx="12" cy="12" r="1" fill="currentColor" stroke="none"/>',
  clock: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 1.8"/>',
  history: '<path d="M4.4 10.7a7.8 7.8 0 1 1 .7 5.2M4.1 6.7v4.8h4.7m3.3-4v4.6l2.9 1.7"/>',
  plug: '<path d="M9 3v4.5M15 3v4.5"/><path d="M6.5 7.5h11V11a5.5 5.5 0 0 1-11 0Z"/><path d="M12 16.5V21"/>',
  stethoscope: '<path d="M3.5 8V5.5a2 2 0 0 1 2-2H8m8 0h2.5a2 2 0 0 1 2 2V8m0 8v2.5a2 2 0 0 1-2 2H16m-8 0H5.5a2 2 0 0 1-2-2V16M6.8 15h2.6a2 2 0 0 0 1.4-.6l3.4-3.8a2 2 0 0 1 1.4-.6h.7"/><circle cx="17.6" cy="10" r="1.55" fill="currentColor" stroke="none" data-node=""/>',
  download: '<path d="M12 4v11m-4.5-4.5L12 15l4.5-4.5"/><path d="M5 19.5h14"/>',
  timer: '<circle cx="12" cy="13" r="7.5"/><path d="M12 9.5V13l2 2M9.5 3h5"/>',
  shield: '<path d="M12 3.5 19.5 6v5c0 4.3-2.7 7.2-7.5 9.6C7.2 18.2 4.5 15.3 4.5 11V6L12 3.5Z"/><path d="m8.8 12 2.1 2.1 4.4-4.4"/>',
  shieldCheck: '<path d="M12 21s6.5-3.2 6.5-8V6L12 3.5 5.5 6v7c0 4.8 6.5 8 6.5 8Z"/><path d="m9.4 12.2 1.8 1.8 3.5-3.8"/>',
  list: '<path d="M9.5 6.5h10M9.5 12h10M9.5 17.5h10"/><circle cx="5" cy="6.5" r=".9" fill="currentColor"/><circle cx="5" cy="12" r=".9" fill="currentColor"/><circle cx="5" cy="17.5" r=".9" fill="currentColor"/>',
  checkCircle: '<circle cx="12" cy="12" r="8.5"/><path d="m8.3 12.2 2.6 2.6 4.8-5.3"/>',
  xCircle: '<circle cx="12" cy="12" r="8.5"/><path d="m9.3 9.3 5.4 5.4m0-5.4-5.4 5.4"/>',
  help: '<circle cx="12" cy="12" r="8.5"/><path d="M9.7 9.6a2.4 2.4 0 1 1 3.3 2.2c-.6.3-1 .8-1 1.4v.3M12 16.5h.01"/>',
  power: '<path d="M12 3.5V11"/><path d="M6.7 6.7a7.5 7.5 0 1 0 10.6 0"/>',
  link: '<path d="m9.1 14.9 5.8-5.8M8.5 10l-2 2a3.9 3.9 0 0 0 5.5 5.5l2-2m-4-7 2-2a3.9 3.9 0 0 1 5.5 5.5l-2 2"/>',
  trash: '<path d="M4.5 6.5h15"/><path d="M9 6.5V5a1.5 1.5 0 0 1 1.5-1.5h3A1.5 1.5 0 0 1 15 5v1.5"/><path d="m6.5 6.5.8 12a2 2 0 0 0 2 1.9h5.4a2 2 0 0 0 2-1.9l.8-12"/>',
  unlink: '<path d="m8.4 9.9-2.3 2.3a3.7 3.7 0 0 0 5.2 5.2l2.3-2.3m-3.2-6.2 2.3-2.3a3.7 3.7 0 0 1 5.2 5.2l-2.3 2.3M5 5l14 14"/>',
  pull: '<path d="M12 4v10.6m-4.1-4.2 4.1 4.2 4.1-4.2M4.5 13.5v5a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2v-5"/>',
  clone: '<path d="M15 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h1"/><rect x="8.5" y="8.5" width="11.5" height="11.5" rx="2"/><path d="M14.25 11.6v5.3m-2.65-2.65h5.3"/>',
  users: '<circle cx="9" cy="7.6" r="3"/><path d="M3.4 19.5v-1c0-2.7 2.4-4.7 5.6-4.7s5.6 2 5.6 4.7v1m.1-14.2a2.7 2.7 0 0 1 0 5.3m1.3 3.2c2.5.3 4.3 2.1 4.3 4.6v1.1"/>',
  offline: '<path d="M7.2 18.5H6.6a3.7 3.7 0 0 1-1.2-7.2m3-3.8a5.7 5.7 0 0 1 8.6 2.1 4.5 4.5 0 0 1 2.3 8.3M4 4l16 16"/>',
  editor: '<rect x="3.3" y="4.7" width="17.4" height="14.6" rx="2.4"/><path d="M3.3 8.1h17.4m-10.4 2.9-2.8 2.7 2.8 2.7m3.4-5.4 2.8 2.7-2.8 2.7"/>',
  command: '<path d="M9 9H6.5A2.5 2.5 0 1 1 9 6.5V9h6V6.5A2.5 2.5 0 1 1 17.5 9H15v6h2.5A2.5 2.5 0 1 1 15 17.5V15H9v2.5A2.5 2.5 0 1 1 6.5 15H9Z"/>',
  filter: '<path d="M3.8 5.2h16.4L14.1 12v5.2l-4.2 2.1V12l-6.1-6.8Z"/>',
  undo: '<path d="M4.5 11H14a5 5 0 0 1 0 10h-2.3M8.7 6.8 4.5 11l4.2 4.2"/>',
  pullRequest: '<path d="M6 7.2v9.6m5.5-11.3h1.8a4 4 0 0 1 4 4v3.3m-2.5-2.3 2.5 2.5 2.5-2.5"/><circle cx="6" cy="5.4" r="1.8"/><circle cx="6" cy="18.6" r="1.8"/><circle cx="17.3" cy="18.6" r="1.8" fill="currentColor" stroke="none" data-node=""/>',
  /** Marca GitHub (preenchido, uso em fundo escuro). */
  githubMark:
    '<path fill="currentColor" stroke="none" d="M12 1.9C6.5 1.9 2 6.4 2 11.9c0 4.3 2.8 8 6.8 9.3.5.1.7-.2.7-.5v-1.6c-2.8.6-3.4-1.3-3.4-1.3-.5-1.1-1.1-1.5-1.1-1.5-.9-.6.1-.6.1-.6 1 .1 1.5 1 1.5 1 .9 1.5 2.4 1.1 3 .8.1-.6.4-1 .6-1.2-2.2-.2-4.6-1.1-4.6-5 0-1.1.4-2 1-2.7-.1-.2-.4-1.2.1-2.5 0 0 .8-.3 2.7 1 .8-.2 1.6-.3 2.5-.3s1.7.1 2.5.3c1.9-1.3 2.7-1 2.7-1 .5 1.3.2 2.3.1 2.5.6.7 1 1.6 1 2.7 0 3.9-2.3 4.7-4.6 5 .4.3.7.9.7 1.8v2.6c0 .3.2.6.7.5 4-1.3 6.8-5 6.8-9.3 0-5.5-4.5-10-10-10Z"/>',
  /** Marca Bitbucket (balde estilizado). */
  bitbucketMark:
    '<path fill="currentColor" stroke="none" d="M4.8 3.2h9.2l1.1 6.6H6.9L5.6 19.5h12.8l2.1-12.5H9.8L8.7 3.2Z"/>',
} as const;

export type IconName = keyof typeof paths;

export function icon(name: IconName, size = 16): string {
  return `<svg class="icon" data-gitpass-icon="${name}" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">${paths[name]}</svg>`;
}

/** Só o desenho interno (para compor dentro de outro `<svg>`). */
export function iconPaths(name: IconName): string {
  return paths[name];
}

/** Marca de estado para selos e listas: sempre ícone + texto, nunca só cor. */
export function stateIcon(kind: "success" | "warning" | "error" | "info" | "neutral", size = 12): string {
  const name: IconName = kind === "success" ? "check" : kind === "error" ? "xCircle" : kind === "warning" ? "alert" : kind === "info" ? "info" : "help";
  return icon(name, size);
}

/** Símbolo da marca (ícone do app na barra lateral e nas boas-vindas). */
export function brandSymbol(): string {
  return `<span class="brand-icon" aria-hidden="true">${icon("logo", 28)}</span>`;
}

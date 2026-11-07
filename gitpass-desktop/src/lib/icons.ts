/** Ícones de traço 24×24 (traço 1.7), herdam `currentColor`. */
const paths = {
  logo: '<path d="M6.5 7.3 12 12.8l5.5-5.5M12 12.8v4.1" stroke-width="2.25"/><circle cx="6" cy="6.5" r="2.45" fill="currentColor" stroke="none"/><circle cx="18" cy="6.5" r="2.45" fill="currentColor" stroke="none"/><circle cx="12" cy="18.3" r="2.45" fill="currentColor" stroke="none"/>',
  dashboard: '<rect x="3.5" y="3.5" width="7" height="7" rx="1.5"/><rect x="13.5" y="3.5" width="7" height="7" rx="1.5"/><rect x="3.5" y="13.5" width="7" height="7" rx="1.5"/><rect x="13.5" y="13.5" width="7" height="7" rx="1.5"/>',
  repo: '<path d="M6.5 18a2 2 0 0 1 2-2H19M6.5 18a2 2 0 0 0 2 2H19V4H8.5a2 2 0 0 0-2 2v12ZM10 8h5M10 11h3"/>',
  user: '<circle cx="10" cy="8" r="3"/><path d="M4 20v-1.5a4 4 0 0 1 4-4h4a4 4 0 0 1 4 4V20M17 5.5a3 3 0 0 1 0 5.8M20.5 20v-1.5a4 4 0 0 0-2.2-3.6"/>',
  cloud: '<path d="M6.5 18.5h11a4 4 0 0 0 .4-8 5.7 5.7 0 0 0-10.8-.8h-.6a4.4 4.4 0 0 0 0 8.8Z"/>',
  settings: '<path d="m9.7 4 .5-1.3h3.6l.5 1.3.5 1.3 1.1.7 1.4-.2 1.5-.2 1.8 3.2-.9 1.1-.9 1.1v1.4l.9 1.1.9 1.1-1.8 3.2-1.5-.2-1.4-.2-1.1.7-.5 1.3-.5 1.3h-3.6l-.5-1.3-.5-1.3-1.1-.7-1.4.2-1.5.2-1.8-3.2.9-1.1.9-1.1V11l-.9-1.1-.9-1.1 1.8-3.2 1.5.2 1.4.2 1.1-.7Z"/><circle cx="12" cy="12" r="3"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  refresh: '<path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3"/><path d="M19.5 4.5v4.5H15"/>',
  folder: '<path d="M3.5 7a2 2 0 0 1 2-2H9l2 2.3h7.5a2 2 0 0 1 2 2v7.2a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2Z"/>',
  external: '<path d="M13 4.5h6.5V11m0-6.5L10 14m7.5-1v4.5a2 2 0 0 1-2 2h-9a2 2 0 0 1-2-2v-9a2 2 0 0 1 2-2H11"/>',
  copy: '<rect x="8.5" y="8.5" width="11" height="11" rx="2"/><path d="M15.5 8.5v-2a2 2 0 0 0-2-2h-7a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h2"/>',
  check: '<path d="m5.5 12 4 4 9-9"/>',
  alert: '<path d="m10.4 4.7-7.1 12.7a1.7 1.7 0 0 0 1.5 2.6h14.4a1.7 1.7 0 0 0 1.5-2.6L13.6 4.7a1.8 1.8 0 0 0-3.2 0Z"/><path d="M12 9v4m0 3h.01"/>',
  x: '<path d="m6.5 6.5 11 11m-11 0 11-11"/>',
  search: '<circle cx="10.5" cy="10.5" r="5.5"/><path d="m15 15 4 4"/>',
  lock: '<rect x="5" y="10.5" width="14" height="9.5" rx="2"/><path d="M8.5 10.5V8a3.5 3.5 0 0 1 7 0v2.5"/>',
  key: '<circle cx="8" cy="15.5" r="3.8"/><path d="m10.8 12.8 8.2-8.2M16.5 7l2.5 2.5M14.5 9l1.8 1.8"/>',
  branch: '<circle cx="7" cy="5" r="2"/><circle cx="17" cy="7" r="2"/><circle cx="7" cy="19" r="2"/><path d="M7 7v10m10-8c0 5-4 5-6 5a4 4 0 0 0-4 3"/>',
  commit: '<circle cx="12" cy="12" r="3.5"/><path d="M3.5 12h5m7 0h5"/>',
  merge: '<circle cx="17" cy="17" r="2.3"/><circle cx="7" cy="6" r="2.3"/><path d="M7 20V8.3a8.5 8.5 0 0 0 7.7 8.7"/>',
  sun: '<circle cx="12" cy="12" r="3.5"/><path d="M12 2.5v2m0 15v2M2.5 12h2m15 0h2M5.3 5.3l1.4 1.4m10.6 10.6 1.4 1.4M5.3 18.7l1.4-1.4M17.3 6.7l1.4-1.4"/>',
  moon: '<path d="M20.2 15.5a8.8 8.8 0 0 1-11.7-11.7A8.8 8.8 0 1 0 20.2 15.5Z"/>',
  monitor: '<rect x="3" y="4.5" width="18" height="12.5" rx="2"/><path d="M12 17v3m-4 0h8"/>',
  activity: '<path d="M20.5 12h-3.5l-2.6 7.5L9.6 4.5 7 12H3.5"/>',
  arrowUp: '<path d="M12 19V5.5M6 11.5l6-6 6 6"/>',
  arrowDown: '<path d="M12 5v13.5M18 12.5l-6 6-6-6"/>',
  back: '<path d="m10 6-6 6 6 6M4 12h15"/>',
  chevronLeft: '<path d="m15 6-6 6 6 6"/>',
  chevronRight: '<path d="m9 6 6 6-6 6"/>',
  chevronDown: '<path d="m6 9 6 6 6-6"/>',
  more: '<circle cx="5.5" cy="12" r="1.2" fill="currentColor" stroke="none"/><circle cx="12" cy="12" r="1.2" fill="currentColor" stroke="none"/><circle cx="18.5" cy="12" r="1.2" fill="currentColor" stroke="none"/>',
  file: '<path d="M13.5 3.5h-6a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h9a2 2 0 0 0 2-2V8.5Z"/><path d="M13.5 3.5v5h5"/>',
  info: '<circle cx="12" cy="12" r="8.5"/><path d="M12 11.5v4m0-7h.01"/>',
  edit: '<path d="M12.5 19.5h7"/><path d="M15.8 4.7a2 2 0 0 1 2.9 2.9L8.2 18.1l-3.7.9.9-3.7Z"/>',
  terminal: '<rect x="3" y="5" width="18" height="14" rx="2.5"/><path d="m7 9 3 3-3 3m6 0h4"/>',
  send: '<path d="M20 4 10.5 13.5"/><path d="M20 4 14 20l-3.5-6.5L4 10Z"/>',
  pin: '<path d="M12 16.5V21"/><path d="M9.2 3.5h5.6l-.9 5.2 2.9 2.9v2H7.2v-2l2.9-2.9Z"/>',
  pinOff: '<path d="M12 16.5V21"/><path d="M14.8 3.5H9.2l.5 3M13.9 8.7l2.9 2.9v2h-3M10.2 13.6H7.2v-2l2.2-2.2"/><path d="m4 4 16 16"/>',
  target: '<circle cx="12" cy="12" r="8.5"/><circle cx="12" cy="12" r="4.5"/><circle cx="12" cy="12" r="1" fill="currentColor" stroke="none"/>',
  clock: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 1.8"/>',
  history: '<path d="M4 11.5a8 8 0 1 1 1.7 5M4 6v5.5h5.5M12 8v4l3 1.5"/>',
  plug: '<path d="M9 3v4.5M15 3v4.5"/><path d="M6.5 7.5h11V11a5.5 5.5 0 0 1-11 0Z"/><path d="M12 16.5V21"/>',
  stethoscope: '<path d="M5.5 3.5v4.5a4.5 4.5 0 0 0 9 0V3.5"/><path d="M10 12.5v2a4.8 4.8 0 0 0 9.5 0V12"/><circle cx="19.5" cy="10" r="2"/>',
  download: '<path d="M12 4v11m-4.5-4.5L12 15l4.5-4.5"/><path d="M5 19.5h14"/>',
  timer: '<circle cx="12" cy="13" r="7.5"/><path d="M12 9.5V13l2 2M9.5 3h5"/>',
  shield: '<path d="M12 21s6.5-3.2 6.5-8V6L12 3.5 5.5 6v7c0 4.8 6.5 8 6.5 8Z"/>',
  shieldCheck: '<path d="M12 21s6.5-3.2 6.5-8V6L12 3.5 5.5 6v7c0 4.8 6.5 8 6.5 8Z"/><path d="m9.4 12.2 1.8 1.8 3.5-3.8"/>',
  list: '<path d="M9.5 6.5h10M9.5 12h10M9.5 17.5h10"/><circle cx="5" cy="6.5" r=".9" fill="currentColor"/><circle cx="5" cy="12" r=".9" fill="currentColor"/><circle cx="5" cy="17.5" r=".9" fill="currentColor"/>',
  checkCircle: '<circle cx="12" cy="12" r="8.5"/><path d="m8.3 12.2 2.6 2.6 4.8-5.3"/>',
  xCircle: '<circle cx="12" cy="12" r="8.5"/><path d="m9.3 9.3 5.4 5.4m0-5.4-5.4 5.4"/>',
  help: '<circle cx="12" cy="12" r="8.5"/><path d="M9.7 9.6a2.4 2.4 0 1 1 3.3 2.2c-.6.3-1 .8-1 1.4v.3M12 16.5h.01"/>',
  power: '<path d="M12 3.5V11"/><path d="M6.7 6.7a7.5 7.5 0 1 0 10.6 0"/>',
  link: '<path d="m10 14 4-4m-6.5.5-2 2a4.2 4.2 0 0 0 6 6l2-2m-3-9 2-2a4.2 4.2 0 0 1 6 6l-2 2"/>',
  trash: '<path d="M4.5 6.5h15"/><path d="M9 6.5V5a1.5 1.5 0 0 1 1.5-1.5h3A1.5 1.5 0 0 1 15 5v1.5"/><path d="m6.5 6.5.8 12a2 2 0 0 0 2 1.9h5.4a2 2 0 0 0 2-1.9l.8-12"/>',
} as const;

export type IconName = keyof typeof paths;

export function icon(name: IconName, size = 16): string {
  return `<svg class="icon" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">${paths[name]}</svg>`;
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
  return `<span class="brand-icon" aria-hidden="true">${icon("logo", 26)}</span>`;
}

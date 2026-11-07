/**
 * Popover em vidro com seta, preso a um botão. Um de cada vez; Esc ou clique fora fecha e devolve o foco.
 * O Tab circula dentro do popover enquanto ele está aberto.
 */

export type PopoverHandle = { el: HTMLElement; close: () => void; reposition: () => void };

type PopoverOptions = {
  /** Nome acessível do diálogo. */
  label: string;
  html: string;
  className?: string;
  /** `bottom`: abaixo do botão, alinhado à esquerda. */
  placement?: "bottom" | "right";
  /** Em `bottom`, `end` alinha a borda direita do popover à do botão. */
  align?: "start" | "end";
  onClose?: () => void;
};

let active: { anchor: HTMLElement; handle: PopoverHandle } | null = null;

export function closePopover(): void {
  active?.handle.close();
}

export function isPopoverOpenFor(anchor: HTMLElement): boolean {
  return active?.anchor === anchor;
}

const FOCUSABLE = "button:not([disabled]), a[href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex='-1'])";

export function openPopover(anchor: HTMLElement, opts: PopoverOptions): PopoverHandle {
  closePopover();
  const placement = opts.placement ?? "bottom";
  const el = document.createElement("div");
  el.className = `gp-popover gp-popover-${placement}${opts.className ? ` ${opts.className}` : ""}`;
  el.setAttribute("role", "dialog");
  el.setAttribute("aria-label", opts.label);
  el.innerHTML = `<span class="gp-popover-arrow" aria-hidden="true"></span><div class="gp-popover-body">${opts.html}</div>`;
  document.body.appendChild(el);
  anchor.setAttribute("aria-expanded", "true");

  const reposition = (): void => {
    const r = anchor.getBoundingClientRect();
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    const margin = 8;
    let left: number;
    let top: number;
    if (placement === "right") {
      left = r.right + 10;
      top = Math.min(Math.max(margin, r.top + r.height / 2 - 28), window.innerHeight - h - margin);
      el.style.setProperty("--gp-arrow-offset", `${Math.max(14, r.top + r.height / 2 - top)}px`);
    } else {
      const wanted = opts.align === "end" ? r.right - w : r.left;
      left = Math.min(Math.max(margin, wanted), window.innerWidth - w - margin);
      top = r.bottom + 10;
      if (top + h > window.innerHeight - margin) top = Math.max(margin, window.innerHeight - h - margin);
      const point = opts.align === "end" ? r.right - Math.min(r.width / 2, 36) : r.left + Math.min(r.width / 2, 36);
      el.style.setProperty("--gp-arrow-offset", `${Math.min(Math.max(16, point - left), w - 16)}px`);
    }
    el.style.left = `${Math.round(left)}px`;
    el.style.top = `${Math.round(top)}px`;
  };

  const onDocDown = (e: PointerEvent): void => {
    const t = e.target as Node;
    if (!el.contains(t) && !anchor.contains(t)) handle.close();
  };
  const onKey = (e: KeyboardEvent): void => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      handle.close();
      anchor.focus();
    } else if (e.key === "Tab") {
      const items = Array.from(el.querySelectorAll<HTMLElement>(FOCUSABLE));
      if (!items.length) return;
      const first = items[0];
      const last = items[items.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  };
  const onResize = (): void => reposition();

  const handle: PopoverHandle = {
    el,
    reposition,
    close: () => {
      if (!el.isConnected) return;
      document.removeEventListener("pointerdown", onDocDown, true);
      el.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", onResize);
      el.remove();
      if (anchor.isConnected) anchor.setAttribute("aria-expanded", "false");
      if (active?.handle === handle) active = null;
      opts.onClose?.();
    },
  };

  reposition();
  document.addEventListener("pointerdown", onDocDown, true);
  el.addEventListener("keydown", onKey);
  window.addEventListener("resize", onResize);
  active = { anchor, handle };
  (el.querySelector<HTMLElement>("[data-autofocus]") ?? el.querySelector<HTMLElement>(FOCUSABLE))?.focus();
  return handle;
}

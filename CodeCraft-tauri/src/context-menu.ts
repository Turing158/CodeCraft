export type ContextMenuIcon =
  | "arrow-up-right"
  | "copy"
  | "refresh"
  | "settings"
  | "minus"
  | "x"
  | "trash"
  | "info";

export type ContextMenuCloseReason =
  | "selection"
  | "outside"
  | "escape"
  | "tab"
  | "programmatic";

export interface ContextMenuContext<T = unknown> {
  event: MouseEvent;
  target: HTMLElement;
  data: T | undefined;
  close: () => void;
}

export interface ContextMenuAction<T = unknown> {
  type?: "item";
  id: string;
  label: string;
  icon?: ContextMenuIcon;
  shortcut?: string;
  danger?: boolean;
  disabled?: boolean | ((context: ContextMenuContext<T>) => boolean);
  visible?: boolean | ((context: ContextMenuContext<T>) => boolean);
  onSelect?: (context: ContextMenuContext<T>) => void | Promise<void>;
}

export interface ContextMenuSeparator<T = unknown> {
  type: "separator";
  id?: string;
  visible?: boolean | ((context: ContextMenuContext<T>) => boolean);
}

export type ContextMenuItem<T = unknown> =
  | ContextMenuAction<T>
  | ContextMenuSeparator<T>;

export interface ContextMenuOptions<T = unknown> {
  items: ContextMenuItem<T>[] | ((context: ContextMenuContext<T>) => ContextMenuItem<T>[]);
  getData?: (target: HTMLElement, event: MouseEvent) => T | undefined;
  ariaLabel?: string;
  className?: string;
  closeDurationMs?: number;
  onClose?: (reason: ContextMenuCloseReason) => void;
}

interface RenderedAction<T> {
  action: ContextMenuAction<T>;
  button: HTMLButtonElement;
  context: ContextMenuContext<T>;
}

const DEFAULT_CLOSE_DURATION_MS = 180;

const iconPath: Record<ContextMenuIcon, string> = {
  "arrow-up-right": "M14 4h6v6 M20 4 11 13 M19 13v5a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h5",
  copy: "M8 8h10a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H10a2 2 0 0 1-2-2V8Z M6 16H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2v1",
  refresh: "M20 11a8 8 0 0 0-14.8-4L3 10 M3 5v5h5 M4 13a8 8 0 0 0 14.8 4L21 14 M21 19v-5h-5",
  settings: "M12 3v2 M12 19v2 M5.6 5.6 7 7 M17 17l1.4 1.4 M3 12h2 M19 12h2 M5.6 18.4 7 17 M17 7l1.4-1.4 M16.5 12a4.5 4.5 0 1 1-9 0 4.5 4.5 0 0 1 9 0Z",
  minus: "M5 12h14",
  x: "m6 6 12 12 M18 6 6 18",
  trash: "M3 6h18 M8 6V4h8v2 M19 6l-1 14H6L5 6 M10 11v5 M14 11v5",
  info: "M12 11v5 M12 8h.01 M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z",
};

const isAction = <T>(item: ContextMenuItem<T>): item is ContextMenuAction<T> =>
  item.type !== "separator";

const isVisible = <T>(
  item: ContextMenuItem<T>,
  context: ContextMenuContext<T>,
): boolean => {
  if (item.visible === undefined) return true;
  return typeof item.visible === "function"
    ? item.visible(context)
    : item.visible;
};

const isDisabled = <T>(
  item: ContextMenuAction<T>,
  context: ContextMenuContext<T>,
): boolean => {
  if (item.disabled === undefined) return false;
  return typeof item.disabled === "function"
    ? item.disabled(context)
    : item.disabled;
};

export const compactContextMenuItems = <T>(
  items: ContextMenuItem<T>[],
): ContextMenuItem<T>[] => {
  const compacted: ContextMenuItem<T>[] = [];
  for (const item of items) {
    if (
      item.type === "separator" &&
      (compacted.length === 0 || compacted[compacted.length - 1]?.type === "separator")
    ) {
      continue;
    }
    compacted.push(item);
  }
  if (compacted[compacted.length - 1]?.type === "separator") compacted.pop();
  return compacted;
};

/**
 * A small, framework-free context menu. The item resolver is evaluated for
 * every right-click, so callers can change the menu based on the clicked
 * element without creating another component instance.
 */
export class ContextMenuController<T = unknown> {
  private readonly target: HTMLElement;
  private readonly options: ContextMenuOptions<T>;
  private readonly menu: HTMLDivElement;
  private readonly closeDurationMs: number;
  private closeTimer: ReturnType<typeof setTimeout> | undefined;
  private closeReason: ContextMenuCloseReason = "programmatic";
  private openFrame: number | undefined;
  private lastFocusedElement: HTMLElement | undefined;
  private renderedActions: RenderedAction<T>[] = [];
  private context: ContextMenuContext<T> | undefined;
  private destroyed = false;

  constructor(target: HTMLElement, options: ContextMenuOptions<T>) {
    this.target = target;
    this.options = options;
    this.closeDurationMs = options.closeDurationMs ?? DEFAULT_CLOSE_DURATION_MS;
    this.menu = document.createElement("div");
    this.menu.className = ["context-menu", options.className]
      .filter(Boolean)
      .join(" ");
    this.menu.hidden = true;
    this.menu.setAttribute("role", "menu");
    this.menu.setAttribute("aria-label", options.ariaLabel ?? "快捷菜单");
    this.menu.tabIndex = -1;
    document.body.append(this.menu);

    target.addEventListener("contextmenu", this.handleContextMenu);
    document.addEventListener("pointerdown", this.handleDocumentPointerDown, true);
    document.addEventListener("keydown", this.handleDocumentKeyDown);
    window.addEventListener("resize", this.handleViewportChange);
    window.addEventListener("scroll", this.handleViewportChange, true);
  }

  get element(): HTMLDivElement {
    return this.menu;
  }

  open(event: MouseEvent, target = this.resolveTarget(event.target)): void {
    if (this.destroyed || !target) return;
    event.preventDefault();
    const data = this.options.getData?.(target, event);
    const context: ContextMenuContext<T> = {
      event,
      target,
      data,
      close: () => this.close(),
    };
    this.context = context;
    this.lastFocusedElement =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : undefined;
    this.render(context);
    if (this.renderedActions.length === 0) {
      this.close(false, "programmatic");
      return;
    }

    if (this.closeTimer !== undefined) {
      clearTimeout(this.closeTimer);
      this.closeTimer = undefined;
    }
    if (this.openFrame !== undefined) {
      cancelAnimationFrame(this.openFrame);
    }
    this.menu.hidden = false;
    this.menu.dataset.state = "closed";
    this.position(event.clientX, event.clientY);
    this.openFrame = requestAnimationFrame(() => {
      this.openFrame = undefined;
      if (this.destroyed) return;
      this.menu.dataset.state = "open";
      this.focusFirstEnabled();
    });
  }

  close(
    restoreFocus = false,
    reason: ContextMenuCloseReason = "programmatic",
  ): void {
    if (this.menu.hidden) return;
    if (this.openFrame !== undefined) {
      cancelAnimationFrame(this.openFrame);
      this.openFrame = undefined;
    }
    this.menu.dataset.state = "closing";
    this.menu.setAttribute("aria-hidden", "true");
    this.closeReason = reason;
    if (restoreFocus) {
      this.lastFocusedElement?.focus({ preventScroll: true });
    }
    if (this.closeTimer !== undefined) clearTimeout(this.closeTimer);
    this.closeTimer = setTimeout(() => {
      this.menu.hidden = true;
      this.menu.dataset.state = "closed";
      this.menu.removeAttribute("aria-hidden");
      this.closeTimer = undefined;
      this.context = undefined;
      this.options.onClose?.(this.closeReason);
    }, this.closeDurationMs);
  }

  destroy(): void {
    if (this.destroyed) return;
    this.destroyed = true;
    if (this.closeTimer !== undefined) clearTimeout(this.closeTimer);
    if (this.openFrame !== undefined) cancelAnimationFrame(this.openFrame);
    this.target.removeEventListener("contextmenu", this.handleContextMenu);
    document.removeEventListener("pointerdown", this.handleDocumentPointerDown, true);
    document.removeEventListener("keydown", this.handleDocumentKeyDown);
    window.removeEventListener("resize", this.handleViewportChange);
    window.removeEventListener("scroll", this.handleViewportChange, true);
    this.menu.remove();
  }

  private readonly handleContextMenu = (event: MouseEvent): void => {
    event.preventDefault();
    if (this.menu.contains(event.target as Node | null)) return;
    this.open(event);
  };

  private readonly handleDocumentPointerDown = (event: PointerEvent): void => {
    if (this.menu.hidden || this.menu.contains(event.target as Node | null)) return;
    this.close(false, "outside");
  };

  private readonly handleDocumentKeyDown = (event: KeyboardEvent): void => {
    if (this.menu.hidden) return;
    if (event.key === "Escape") {
      event.preventDefault();
      this.close(true, "escape");
      return;
    }
    if (event.key === "Tab") {
      this.close(false, "tab");
      return;
    }
    const enabled = this.renderedActions.filter(({ button }) => !button.disabled);
    if (enabled.length === 0) return;
    const currentIndex = enabled.findIndex(
      ({ button }) => button === document.activeElement,
    );
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const offset = event.key === "ArrowDown" ? 1 : -1;
      enabled[(currentIndex + offset + enabled.length) % enabled.length]?.button.focus({
        preventScroll: true,
      });
    } else if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      (event.key === "Home" ? enabled[0] : enabled[enabled.length - 1])?.button.focus({
        preventScroll: true,
      });
    }
  };

  private readonly handleViewportChange = (): void => {
    if (this.menu.hidden || !this.context) return;
    this.position(this.context.event.clientX, this.context.event.clientY);
  };

  private resolveTarget(rawTarget: EventTarget | null): HTMLElement | undefined {
    if (rawTarget instanceof HTMLElement) {
      return this.target.contains(rawTarget) ? rawTarget : this.target;
    }
    return this.target;
  }

  private render(context: ContextMenuContext<T>): void {
    const visibleItems = (typeof this.options.items === "function"
      ? this.options.items(context)
      : this.options.items
    ).filter((item) => isVisible(item, context));
    const resolvedItems = compactContextMenuItems(visibleItems);
    const fragment = document.createDocumentFragment();
    this.renderedActions = [];

    for (const item of resolvedItems) {
      if (!isAction(item)) {
        const separator = document.createElement("div");
        separator.className = "context-menu__separator";
        separator.setAttribute("role", "separator");
        fragment.append(separator);
        continue;
      }

      const button = document.createElement("button");
      button.className = "context-menu__item";
      button.type = "button";
      button.setAttribute("role", "menuitem");
      button.dataset.itemId = item.id;
      button.disabled = isDisabled(item, context);
      if (item.danger) button.dataset.variant = "danger";

      if (item.icon) button.append(this.createIcon(item.icon));
      const label = document.createElement("span");
      label.className = "context-menu__label";
      label.textContent = item.label;
      button.append(label);
      if (item.shortcut) {
        const shortcut = document.createElement("kbd");
        shortcut.className = "context-menu__shortcut";
        shortcut.textContent = item.shortcut;
        button.append(shortcut);
      }

      button.addEventListener("click", () => {
        if (button.disabled) return;
        this.close(true, "selection");
        void item.onSelect?.(context);
      });
      fragment.append(button);
      this.renderedActions.push({ action: item, button, context });
    }

    this.menu.replaceChildren(fragment);
    this.menu.removeAttribute("aria-hidden");
  }

  private createIcon(icon: ContextMenuIcon): SVGSVGElement {
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.classList.add("context-menu__icon");
    svg.setAttribute("viewBox", "0 0 24 24");
    svg.setAttribute("fill", "none");
    svg.setAttribute("stroke", "currentColor");
    svg.setAttribute("stroke-width", "1.8");
    svg.setAttribute("stroke-linecap", "round");
    svg.setAttribute("stroke-linejoin", "round");
    svg.setAttribute("aria-hidden", "true");
    for (const pathData of iconPath[icon].split(" M")) {
      const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
      path.setAttribute("d", pathData.startsWith("M") ? pathData : `M${pathData}`);
      svg.append(path);
    }
    return svg;
  }

  private position(clientX: number, clientY: number): void {
    const margin = 8;
    const width = this.menu.offsetWidth;
    const height = this.menu.offsetHeight;
    const left = Math.max(margin, Math.min(clientX, window.innerWidth - width - margin));
    const top = Math.max(margin, Math.min(clientY, window.innerHeight - height - margin));
    this.menu.style.left = `${left}px`;
    this.menu.style.top = `${top}px`;
  }

  private focusFirstEnabled(): void {
    this.renderedActions.find(({ button }) => !button.disabled)?.button.focus({
      preventScroll: true,
    });
  }
}

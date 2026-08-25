export type PanelVisualState = "expanded" | "collapsing" | "collapsed";
export type PanelCollapseSource = "auto" | "manual" | "mini";

export interface PanelPlatform {
  setNativeExpanded(expanded: boolean, animateHeight?: boolean): Promise<void>;
  setVisualState(state: PanelVisualState): void;
  setCollapseSource(source: PanelCollapseSource): void;
}

const DEFAULT_COLLAPSE_DELAY_MS = 250;

export class PanelController {
  private generation = 0;
  private collapseDelay: ReturnType<typeof setTimeout> | undefined;
  private collapseDelayMs = DEFAULT_COLLAPSE_DELAY_MS;
  private visualState: PanelVisualState = "collapsed";

  constructor(private readonly platform: PanelPlatform) {}

  setCollapseDelay(delayMs: number): void {
    if (!Number.isFinite(delayMs)) return;
    this.collapseDelayMs = Math.max(0, delayMs);
  }

  async pointerEntered(): Promise<void> {
    const generation = ++this.generation;
    this.clearTimers();

    this.setVisualState("expanded");
    await this.platform.setNativeExpanded(true);
    if (generation !== this.generation) return;
  }

  async revealIfCollapsed(): Promise<boolean> {
    if (this.visualState === "expanded") return false;

    await this.pointerEntered();
    return true;
  }

  async revealTemporarily(durationMs: number): Promise<void> {
    const generation = ++this.generation;
    this.clearTimers();

    this.setVisualState("expanded");
    this.collapseDelay = setTimeout(() => {
      void this.finishCollapse(generation, "auto");
    }, Math.max(0, durationMs));

    await this.platform.setNativeExpanded(true);
    if (generation !== this.generation) return;
  }

  pointerLeft(): void {
    this.scheduleCollapse("auto");
  }

  collapse(source: PanelCollapseSource = "manual"): Promise<void> {
    const generation = ++this.generation;
    this.clearTimers();

    return this.finishCollapse(generation, source);
  }

  async contentResized(animateHeight = false): Promise<void> {
    if (this.visualState !== "expanded") return;

    await this.platform.setNativeExpanded(true, animateHeight);
  }

  dispose(): void {
    ++this.generation;
    this.clearTimers();
  }

  private clearTimers(): void {
    if (this.collapseDelay !== undefined) {
      clearTimeout(this.collapseDelay);
      this.collapseDelay = undefined;
    }
  }

  private scheduleCollapse(source: PanelCollapseSource): void {
    const generation = ++this.generation;
    this.clearTimers();

    this.collapseDelay = setTimeout(() => {
      void this.finishCollapse(generation, source);
    }, this.collapseDelayMs);
  }

  private setVisualState(state: PanelVisualState): void {
    this.visualState = state;
    this.platform.setVisualState(state);
  }

  private async finishCollapse(
    generation: number,
    source: PanelCollapseSource,
  ): Promise<void> {
    if (generation !== this.generation) return;

    this.platform.setCollapseSource(source);
    this.setVisualState("collapsing");
    const collapsed = await this.collapseNativeWindow(generation);
    if (generation !== this.generation) return;

    if (!collapsed) {
      this.setVisualState("expanded");
      return;
    }

    this.setVisualState("collapsed");
  }

  private async collapseNativeWindow(generation: number): Promise<boolean> {
    if (generation !== this.generation) return false;

    try {
      await this.platform.setNativeExpanded(false);
      return generation === this.generation;
    } catch (error: unknown) {
      console.error("Unable to collapse the CodeCraft panel", error);
      return false;
    }
  }
}

const COLLAPSED_HEIGHT = 5;
const DEFAULT_EXPANDED_HEIGHT = 144;
// Keep the outer window silhouette visibly rounded even when the interface is scaled down.
const MIN_VISIBLE_BOTTOM_CORNER_RADIUS = 12;

export interface PanelClipPathOptions {
  collapsedHeight?: number;
  collapsedCornerProgress?: number;
  interfaceScale?: number;
  panelWidth?: number;
}

function interpolate(start: number, end: number, progress: number): number {
  return start + (end - start) * progress;
}

function coordinate(value: number): string {
  return String(Number(value.toFixed(3)));
}

const COLLAPSED_STRIP_CORNER_RADIUS = 1.75;

function collapsedStripClipPath(
  panelWidth: number,
  collapsedHeight: number,
): string {
  const panelInset = Math.min(12, panelWidth / 3);
  const panelRightInset = panelWidth - panelInset;
  const cornerRadius = COLLAPSED_STRIP_CORNER_RADIUS;
  const outerLeft = coordinate(panelInset - cornerRadius);
  const outerLeftControl = coordinate(panelInset - cornerRadius / 2);
  const outerRight = coordinate(panelRightInset + cornerRadius);
  const outerRightControl = coordinate(panelRightInset + cornerRadius / 2);
  const shoulderControlY = coordinate((cornerRadius * 2) / 3);
  const shoulderY = coordinate(cornerRadius);
  const bottom = coordinate(collapsedHeight);

  return `path("M ${outerLeft} 0 C ${outerLeftControl} 0 ${panelInset} ${shoulderControlY} ${panelInset} ${shoulderY} L ${panelInset} ${bottom} L ${panelRightInset} ${bottom} L ${panelRightInset} ${shoulderY} C ${panelRightInset} ${shoulderControlY} ${outerRightControl} 0 ${outerRight} 0 Z")`;
}

export function panelClipPath(
  windowHeight: number,
  contentHeight = DEFAULT_EXPANDED_HEIGHT,
  options: PanelClipPathOptions = {},
): string {
  const requestedPanelWidth = options.panelWidth ?? 500;
  const panelWidth = Number.isFinite(requestedPanelWidth)
    ? Math.max(36, requestedPanelWidth)
    : 500;
  const panelInset = Math.min(12, panelWidth / 3);
  const panelRightInset = panelWidth - panelInset;
  const requestedCollapsedHeight = options.collapsedHeight ?? COLLAPSED_HEIGHT;
  const collapsedHeight = Number.isFinite(requestedCollapsedHeight)
    ? Math.max(COLLAPSED_HEIGHT, requestedCollapsedHeight)
    : COLLAPSED_HEIGHT;
  const safeContentHeight = Number.isFinite(contentHeight)
    ? contentHeight
    : DEFAULT_EXPANDED_HEIGHT;
  const expandedHeight = Math.max(collapsedHeight, safeContentHeight);
  const safeHeight = Number.isFinite(windowHeight)
    ? windowHeight
    : collapsedHeight;
  const height = Math.min(
    expandedHeight,
    Math.max(collapsedHeight, safeHeight),
  );
  const requestedCollapsedCornerProgress =
    options.collapsedCornerProgress ?? 0;
  const collapsedCornerProgress = Number.isFinite(
    requestedCollapsedCornerProgress,
  )
    ? Math.min(1, Math.max(0, requestedCollapsedCornerProgress))
    : 0;
  if (collapsedCornerProgress === 0 && height <= collapsedHeight) {
    return collapsedStripClipPath(panelWidth, collapsedHeight);
  }
  const heightProgress =
    expandedHeight === collapsedHeight
      ? 0
      : (height - collapsedHeight) / (expandedHeight - collapsedHeight);
  const cornerProgress = interpolate(
    collapsedCornerProgress,
    1,
    heightProgress,
  );
  const requestedInterfaceScale = options.interfaceScale ?? 1;
  const interfaceScale = Number.isFinite(requestedInterfaceScale)
    ? Math.max(0.01, requestedInterfaceScale)
    : 1;

  const outerEdge = coordinate(interpolate(panelInset, 0, cornerProgress));
  const outerControl = coordinate(interpolate(panelInset, 7, cornerProgress));
  const shoulderControlY = coordinate(interpolate(0, 4, cornerProgress));
  const shoulderYValue = interpolate(0, 9, cornerProgress);
  const shoulderY = coordinate(shoulderYValue);
  // The lower curve must stay below the upper shoulder. Without this bound a
  // very short content view can produce a self-intersecting clip path, while
  // Windows' native region rasterizer resolves the same path as a rectangle.
  const requestedCornerRadius = Math.max(
    interpolate(4, 6, cornerProgress),
    (MIN_VISIBLE_BOTTOM_CORNER_RADIUS * cornerProgress) / interfaceScale,
  );
  const maxWidthCornerRadius = Math.max(
    0,
    (panelRightInset - panelInset) / 2,
  );
  const cornerRadius = Math.min(
    requestedCornerRadius,
    Math.max(0, height - shoulderYValue),
    maxWidthCornerRadius,
  );
  const lowerCornerY = coordinate(height - cornerRadius);
  const lowerInnerEdge = coordinate(panelInset + cornerRadius);
  const lowerOuterEdge = coordinate(panelRightInset - cornerRadius);
  const rightOuterControl = coordinate(
    interpolate(panelRightInset, panelWidth - 7, cornerProgress),
  );
  const rightOuterEdge = coordinate(
    interpolate(panelRightInset, panelWidth, cornerProgress),
  );
  const bottom = coordinate(height);

  return `path("M ${outerEdge} 0 C ${outerControl} 0 ${panelInset} ${shoulderControlY} ${panelInset} ${shoulderY} L ${panelInset} ${lowerCornerY} Q ${panelInset} ${bottom} ${lowerInnerEdge} ${bottom} L ${lowerOuterEdge} ${bottom} Q ${panelRightInset} ${bottom} ${panelRightInset} ${lowerCornerY} L ${panelRightInset} ${shoulderY} C ${panelRightInset} ${shoulderControlY} ${rightOuterControl} 0 ${rightOuterEdge} 0 Z")`;
}

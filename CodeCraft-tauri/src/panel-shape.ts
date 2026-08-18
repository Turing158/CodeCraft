const COLLAPSED_HEIGHT = 5;
const DEFAULT_EXPANDED_HEIGHT = 144;

export interface PanelClipPathOptions {
  collapsedHeight?: number;
  collapsedCornerProgress?: number;
  panelWidth?: number;
}

function interpolate(start: number, end: number, progress: number): number {
  return start + (end - start) * progress;
}

function coordinate(value: number): string {
  return String(Number(value.toFixed(3)));
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
  const heightProgress =
    expandedHeight === collapsedHeight
      ? 0
      : (height - collapsedHeight) / (expandedHeight - collapsedHeight);
  const requestedCollapsedCornerProgress =
    options.collapsedCornerProgress ?? 0;
  const collapsedCornerProgress = Number.isFinite(
    requestedCollapsedCornerProgress,
  )
    ? Math.min(1, Math.max(0, requestedCollapsedCornerProgress))
    : 0;
  const cornerProgress = interpolate(
    collapsedCornerProgress,
    1,
    heightProgress,
  );

  const outerEdge = coordinate(interpolate(panelInset, 0, cornerProgress));
  const outerControl = coordinate(interpolate(panelInset, 7, cornerProgress));
  const shoulderControlY = coordinate(interpolate(0, 4, cornerProgress));
  const shoulderY = coordinate(interpolate(0, 9, cornerProgress));
  const cornerRadius = interpolate(4, 6, cornerProgress);
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

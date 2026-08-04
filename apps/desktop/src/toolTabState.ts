export type ToolTabStateItem = {
  id: string;
};

export type ClosedToolTabState<T extends ToolTabStateItem> = {
  tabs: T[];
  activeTabId: string | null;
  shouldCollapse: boolean;
};

export function closeToolTabState<T extends ToolTabStateItem>(
  tabs: T[],
  activeTabId: string | null,
  tabId: string,
): ClosedToolTabState<T> {
  const closingIndex = tabs.findIndex((tab) => tab.id === tabId);
  if (closingIndex < 0) {
    return { tabs, activeTabId, shouldCollapse: false };
  }

  const nextTabs = tabs.filter((tab) => tab.id !== tabId);
  if (nextTabs.length === 0) {
    return { tabs: nextTabs, activeTabId: null, shouldCollapse: true };
  }

  if (activeTabId !== tabId) {
    return { tabs: nextTabs, activeTabId, shouldCollapse: false };
  }

  const replacement =
    nextTabs[Math.min(closingIndex, nextTabs.length - 1)] ?? null;
  return {
    tabs: nextTabs,
    activeTabId: replacement?.id ?? null,
    shouldCollapse: false,
  };
}

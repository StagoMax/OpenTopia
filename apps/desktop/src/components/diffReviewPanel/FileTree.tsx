import { useState, type ReactNode } from "react";
import { ChevronDown, ChevronRight, ExternalLink, Folder } from "lucide-react";
import type { DiffTreeNode } from "../../diffReview";
import { IconButton } from "../ui";

export function DiffTreeRow({
  node,
  depth,
  activePath,
  onSelect,
  onOpenFileTab,
}: {
  node: DiffTreeNode;
  depth: number;
  activePath: string | null;
  onSelect(path: string): void;
  onOpenFileTab(path: string): void;
}): ReactNode {
  const [collapsed, setCollapsed] = useState(false);

  if (node.type === "directory") {
    return (
      <div className="diff-review__tree-group" role="group">
        <button
          className="diff-review__tree-row"
          type="button"
          role="treeitem"
          aria-expanded={!collapsed}
          style={{
            paddingLeft: `calc(var(--space-4) + ${depth} * var(--space-6))`,
          }}
          onClick={() => setCollapsed((value) => !value)}
        >
          {collapsed ? <ChevronRight size={13} /> : <ChevronDown size={13} />}
          <Folder size={13} aria-hidden="true" />
          <span className="diff-review__tree-name">{node.name}</span>
        </button>
        {collapsed
          ? null
          : node.children.map((child) => (
              <DiffTreeRow
                key={child.id}
                node={child}
                depth={depth + 1}
                activePath={activePath}
                onSelect={onSelect}
                onOpenFileTab={onOpenFileTab}
              />
            ))}
      </div>
    );
  }

  return (
    <div className="diff-review__tree-file">
      <button
        className="diff-review__tree-row"
        type="button"
        role="treeitem"
        aria-selected={node.path === activePath}
        data-active={node.path === activePath || undefined}
        title={node.path}
        style={{
          paddingLeft: `calc(var(--space-6) + ${depth} * var(--space-6))`,
        }}
        onClick={() => onSelect(node.path)}
      >
        <span className="diff-review__tree-name">{node.name}</span>
        <span className="diff-review__stats">
          <span className="is-addition">+{node.additions}</span>
          <span className="is-deletion">-{node.deletions}</span>
        </span>
      </button>
      <IconButton
        aria-label={`在标签页中打开 ${node.path}`}
        title="在标签页中打开文件"
        size="compact"
        className="diff-review__tree-open"
        onClick={() => onOpenFileTab(node.path)}
      >
        <ExternalLink size={12} />
      </IconButton>
    </div>
  );
}

import { useState, useCallback, type ReactNode } from "react";
import styles from "./TreeView.module.css";

export interface TreeNode {
  id: string;
  label: ReactNode;
  children?: TreeNode[];
  defaultExpanded?: boolean;
}

export interface TreeViewProps {
  nodes: TreeNode[];
  onNodeClick?: (id: string) => void;
  /** When provided, enables controlled expand/collapse mode. Contains IDs of collapsed nodes. */
  collapsedIds?: Set<string>;
  /** Called when the expand/collapse chevron is clicked (controlled mode). Receives the node ID. */
  onToggle?: (id: string) => void;
  /** When provided, the node with this ID will be visually highlighted as selected. */
  selectedId?: string;
  className?: string;
}

interface TreeNodeItemProps {
  node: TreeNode;
  depth: number;
  onNodeClick?: (id: string) => void;
  collapsedIds?: Set<string>;
  onToggle?: (id: string) => void;
  selectedId?: string;
}

function TreeNodeItem({ node, depth, onNodeClick, collapsedIds, onToggle, selectedId }: TreeNodeItemProps) {
  const controlled = collapsedIds !== undefined;
  const [localExpanded, setLocalExpanded] = useState(node.defaultExpanded ?? true);
  const expanded = controlled ? !collapsedIds.has(node.id) : localExpanded;
  const hasChildren = node.children && node.children.length > 0;

  const handleToggle = useCallback(() => {
    if (hasChildren) {
      if (controlled) {
        onToggle?.(node.id);
      } else {
        setLocalExpanded((prev) => !prev);
      }
    }
  }, [hasChildren, controlled, onToggle, node.id]);

  const handleClick = useCallback(() => {
    onNodeClick?.(node.id);
  }, [node.id, onNodeClick]);

  const isSelected = node.id === selectedId;

  return (
    <li className={styles.nodeItem}>
      <div
        className={[styles.nodeRow, isSelected && styles.selected].filter(Boolean).join(" ")}
        style={{ "--tree-depth": depth } as React.CSSProperties}
        onClick={handleClick}
        role="treeitem"
        aria-expanded={hasChildren ? expanded : undefined}
      >
        {Array.from({ length: depth }).map((_, i) => (
          <span key={i} className={styles.indentGuide} style={{ left: `calc(${i} * var(--tree-indent) + var(--tree-indent) / 2)` }} />
        ))}
        <button
          className={[styles.chevron, !hasChildren && styles.hidden].filter(Boolean).join(" ")}
          onClick={(e) => {
            e.stopPropagation();
            handleToggle();
          }}
          tabIndex={-1}
          aria-hidden={!hasChildren}
        >
          {expanded ? "\u25BC" : "\u25B6"}
        </button>
        <span className={styles.nodeLabel}>{node.label}</span>
      </div>
      {hasChildren && expanded && (
        <ul className={styles.children} role="group">
          {node.children!.map((child) => (
            <TreeNodeItem key={child.id} node={child} depth={depth + 1} onNodeClick={onNodeClick} collapsedIds={collapsedIds} onToggle={onToggle} selectedId={selectedId} />
          ))}
        </ul>
      )}
    </li>
  );
}

export function TreeView({ nodes, onNodeClick, collapsedIds, onToggle, selectedId, className }: TreeViewProps) {
  const cls = [styles.tree, className].filter(Boolean).join(" ");

  return (
    <ul className={cls} role="tree">
      {nodes.map((node) => (
        <TreeNodeItem key={node.id} node={node} depth={0} onNodeClick={onNodeClick} collapsedIds={collapsedIds} onToggle={onToggle} selectedId={selectedId} />
      ))}
    </ul>
  );
}

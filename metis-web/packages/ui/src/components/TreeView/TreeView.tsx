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
  className?: string;
}

interface TreeNodeItemProps {
  node: TreeNode;
  depth: number;
  onNodeClick?: (id: string) => void;
}

function TreeNodeItem({ node, depth, onNodeClick }: TreeNodeItemProps) {
  const [expanded, setExpanded] = useState(node.defaultExpanded ?? true);
  const hasChildren = node.children && node.children.length > 0;

  const handleToggle = useCallback(() => {
    if (hasChildren) {
      setExpanded((prev) => !prev);
    }
  }, [hasChildren]);

  const handleClick = useCallback(() => {
    onNodeClick?.(node.id);
  }, [node.id, onNodeClick]);

  return (
    <li className={styles.nodeItem}>
      <div
        className={styles.nodeRow}
        style={{ paddingLeft: `calc(${depth} * var(--tree-indent))` }}
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
            <TreeNodeItem key={child.id} node={child} depth={depth + 1} onNodeClick={onNodeClick} />
          ))}
        </ul>
      )}
    </li>
  );
}

export function TreeView({ nodes, onNodeClick, className }: TreeViewProps) {
  const cls = [styles.tree, className].filter(Boolean).join(" ");

  return (
    <ul className={cls} role="tree">
      {nodes.map((node) => (
        <TreeNodeItem key={node.id} node={node} depth={0} onNodeClick={onNodeClick} />
      ))}
    </ul>
  );
}

// @ts-check
/**
 * Pure helpers over the DiskNode tree of `disk_usage_tree` (SCAN-05/06/08):
 * flatten for the size-sorted tree grid, replace a lazily loaded subtree, look
 * a node up. A node's identity is its `path` (the virtual root has an empty one).
 */

/** @typedef {{ name: string, path: string, is_dir: boolean, size: number, allocated: number, files: number, children: any[], other?: { size: number, count: number } }} DiskNode */

/** Find a node by path. @param {DiskNode | null} root @param {string} path @returns {DiskNode | null} */
export function findNode(root, path) {
  if (!root) return null;
  if (root.path === path) return root;
  for (const c of root.children ?? []) {
    if (path === c.path || path.startsWith(c.path.replace(/[\\/]+$/, '')) ) { const f = findNode(c, path); if (f) return f; }
  }
  return null;
}

/** Immutable replace of the node at `path` with `next`. @param {DiskNode} root @param {string} path @param {DiskNode} next @returns {DiskNode} */
export function replaceNode(root, path, next) {
  if (root.path === path) return next;
  return { ...root, children: (root.children ?? []).map((c) => (path === c.path || path.startsWith(c.path.replace(/[\\/]+$/, '')) ? replaceNode(c, path, next) : c)) };
}

/** A folder that has content but whose children were not returned yet (deeper than the requested depth). @param {DiskNode} n */
export const needsLoad = (n) => n.is_dir && (n.children?.length ?? 0) === 0 && !n.other && n.files > 0 && n.size > 0;

/** Sorted copy of a child list. @param {DiskNode[]} list @param {'size'|'name'} sort */
export const sortChildren = (list, sort) => [...list].sort((a, b) => (sort === 'name' ? a.name.localeCompare(b.name) : b.size - a.size));

/**
 * Rows of the tree grid, depth first, only through expanded nodes.
 * @param {DiskNode | null} root @param {Record<string, boolean>} expanded @param {'size'|'name'} sort
 * @returns {{ node: DiskNode, depth: number, expandable: boolean, open: boolean, share: number, other?: boolean }[]}
 */
export function flatten(root, expanded, sort = 'size') {
  if (!root) return [];
  const out = [];
  const walk = (node, depth, parentSize) => {
    const expandable = node.is_dir && (node.children.length > 0 || needsLoad(node) || !!node.other);
    const open = !!expanded[node.path] && expandable;
    out.push({ node, depth, expandable, open, share: parentSize > 0 ? node.size / parentSize : 0 });
    if (!open) return;
    for (const c of sortChildren(node.children, sort)) walk(c, depth + 1, node.size);
    if (node.other) out.push({ node: { name: '', path: `${node.path}\u0000other`, is_dir: false, size: node.other.size, allocated: node.other.size, files: node.other.count, children: [] }, depth: depth + 1, expandable: false, open: false, share: node.size > 0 ? node.other.size / node.size : 0, other: true });
  };
  for (const c of sortChildren(root.children, sort)) walk(c, 0, root.size);
  return out;
}

/** Tiles for the treemap of a node: its children plus an "other" tile. @param {DiskNode} node */
export function treemapItems(node) {
  const items = (node.children ?? []).map((c) => ({ key: c.path, value: c.size }));
  if (node.other?.size) items.push({ key: `${node.path}\u0000other`, value: node.other.size });
  return items;
}

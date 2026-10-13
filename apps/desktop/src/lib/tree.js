export function buildTree(files) {
  const root = { children: new Map() };
  for (const path of files) {
    const parts = path.split('/');
    let parent = root;
    for (const [index, name] of parts.entries()) {
      const isFile = index === parts.length - 1;
      if (!parent.children.has(name)) {
        parent.children.set(name, {
          kind: isFile ? 'file' : 'folder',
          name,
          path: parts.slice(0, index + 1).join('/'),
          children: new Map(),
        });
      }
      parent = parent.children.get(name);
    }
  }
  function orderedChildren(node) {
    return [...node.children.values()]
      .sort((left, right) => {
        if (left.kind !== right.kind) return left.kind === 'folder' ? -1 : 1;
        return left.name.localeCompare(right.name);
      })
      .map((child) => ({ ...child, children: orderedChildren(child) }));
  }
  return orderedChildren(root);
}

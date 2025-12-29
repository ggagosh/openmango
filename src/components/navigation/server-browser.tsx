import { useCallback, useMemo, useRef, useEffect } from 'react';
import { useKeyboard, useTerminalDimensions } from '@opentui/react';
import type { ScrollBoxRenderable } from '@opentui/core';
import { useApp } from '../../context/app-context.tsx';
import { useFocus } from '../../context/focus-context.tsx';
import { useModal } from '../../context/modal-context.tsx';
import { useConnection } from '../../hooks/use-connection.ts';
import { TreeNode } from './tree-node.tsx';
import { TreeLeaf } from './tree-leaf.tsx';
import { SearchInput } from '../shared/search-input.tsx';
import type { TreeNodeData } from '../../types/index.ts';
import { colors } from '../../theme/index.ts';

interface ServerBrowserProps {
  focused: boolean;
}

function flattenTree(nodes: TreeNodeData[]): TreeNodeData[] {
  const result: TreeNodeData[] = [];
  for (const node of nodes) {
    result.push(node);
    if (node.isExpanded && node.children.length > 0) {
      result.push(...flattenTree(node.children));
    }
  }
  return result;
}

function filterTree(nodes: TreeNodeData[], query: string): TreeNodeData[] {
  if (!query) {
    return nodes;
  }
  const lowerQuery = query.toLowerCase();

  return nodes
    .map((node) => {
      const nameMatches = node.name.toLowerCase().includes(lowerQuery);
      const filteredChildren = filterTree(node.children, query);

      if (nameMatches || filteredChildren.length > 0) {
        return {
          ...node,
          isExpanded: filteredChildren.length > 0 ? true : node.isExpanded,
          children: filteredChildren,
        };
      }
      return null;
    })
    .filter((node): node is TreeNodeData => node !== null);
}

function findParent(nodes: TreeNodeData[], childId: string): TreeNodeData | null {
  for (const node of nodes) {
    for (const child of node.children) {
      if (child.id === childId) {
        return node;
      }
      const found = findParent([child], childId);
      if (found) {
        return found;
      }
    }
  }
  return null;
}

export function ServerBrowser({ focused }: ServerBrowserProps) {
  const { state, dispatch } = useApp();
  const { setActiveZone } = useFocus();
  const { openModal } = useModal();
  const { connect, loadCollections } = useConnection();
  const { height: terminalHeight } = useTerminalDimensions();
  const scrollboxRef = useRef<ScrollBoxRenderable>(null);

  const filteredTree = useMemo(
    () => filterTree(state.navigationTree, state.searchQuery),
    [state.navigationTree, state.searchQuery]
  );

  const flatNodes = useMemo(() => flattenTree(filteredTree), [filteredTree]);

  const currentIndex = useMemo(() => {
    if (!state.selectedNodeId) {
      return -1;
    }
    return flatNodes.findIndex((n) => n.id === state.selectedNodeId);
  }, [flatNodes, state.selectedNodeId]);

  const selectedNode = currentIndex >= 0 ? flatNodes[currentIndex] : null;

  // Calculate visible rows in the scrollbox
  // Terminal height minus: title (1) + footer (1) + borders (2) + search/hint if active
  const chromeHeight =
    4 + (state.isSearching ? 1 : 0) + (!state.isSearching && state.searchQuery ? 1 : 0);
  const visibleRows = Math.max(1, terminalHeight - chromeHeight);

  // Scroll to keep selected item visible
  useEffect(() => {
    const scrollbox = scrollboxRef.current;
    if (!scrollbox || currentIndex < 0) return;

    const scrollTop = scrollbox.scrollTop;
    const scrollBottom = scrollTop + visibleRows - 1;

    // If selected item is above visible area, scroll up
    if (currentIndex < scrollTop) {
      scrollbox.scrollTo(currentIndex);
    }
    // If selected item is below visible area, scroll down
    else if (currentIndex > scrollBottom) {
      scrollbox.scrollTo(currentIndex - visibleRows + 1);
    }
  }, [currentIndex, visibleRows]);

  const moveSelection = useCallback(
    (delta: number) => {
      if (flatNodes.length === 0) {
        return;
      }
      let newIndex = currentIndex + delta;
      if (newIndex < 0) {
        newIndex = 0;
      }
      if (newIndex >= flatNodes.length) {
        newIndex = flatNodes.length - 1;
      }
      dispatch({ payload: flatNodes[newIndex]?.id ?? null, type: 'SELECT_NODE' });
    },
    [flatNodes, currentIndex, dispatch]
  );

  useKeyboard((key) => {
    if (state.isSearching) {
      if (key.name === 'escape') {
        dispatch({ type: 'END_SEARCH' });
        dispatch({ payload: '', type: 'SET_SEARCH' });
      }
      return;
    }

    if (!focused) {
      return;
    }

    if (key.name === '/') {
      dispatch({ type: 'START_SEARCH' });
      return;
    }

    if (key.name === 'escape' && state.searchQuery) {
      dispatch({ payload: '', type: 'SET_SEARCH' });
      return;
    }

    if (key.name === 'up' || key.name === 'k') {
      moveSelection(-1);
    }
    if (key.name === 'down' || key.name === 'j') {
      moveSelection(1);
    }

    if (key.name === 'right' || key.name === 'l') {
      if (selectedNode) {
        // Already expanded with children - move into children
        if (selectedNode.isExpanded && selectedNode.children.length > 0) {
          moveSelection(1);
        }
        // Has children - just expand
        else if (selectedNode.children.length > 0) {
          dispatch({ payload: selectedNode.id, type: 'TOGGLE_EXPAND' });
        }
        // Connection without children - connect to load databases
        else if (
          selectedNode.type === 'connection' &&
          (selectedNode.status === 'disconnected' || selectedNode.status === 'error')
        ) {
          void connect(selectedNode.connectionId!);
        }
        // Database without children - load collections
        else if (selectedNode.type === 'database') {
          dispatch({ payload: selectedNode.id, type: 'TOGGLE_EXPAND' });
          void loadCollections(
            selectedNode.connectionId!,
            selectedNode.databaseName!,
            selectedNode.id
          );
        }
        // Collection - switch to panel (drill into documents)
        else if (selectedNode.type === 'collection') {
          setActiveZone('panel');
        }
      }
    }
    if (key.name === 'left' || key.name === 'h') {
      if (selectedNode) {
        if (selectedNode.isExpanded && selectedNode.children.length > 0) {
          dispatch({ payload: selectedNode.id, type: 'TOGGLE_EXPAND' });
        } else {
          const parent = findParent(filteredTree, selectedNode.id);
          if (parent) {
            dispatch({ payload: parent.id, type: 'SELECT_NODE' });
          }
        }
      }
    }

    if (key.name === 'return' || key.name === 'space') {
      if (selectedNode) {
        // Handle connections - connect if disconnected, toggle if connected
        if (selectedNode.type === 'connection') {
          if (selectedNode.status === 'disconnected' || selectedNode.status === 'error') {
            void connect(selectedNode.connectionId!);
          } else if (selectedNode.children.length > 0) {
            dispatch({ payload: selectedNode.id, type: 'TOGGLE_EXPAND' });
          }
        }
        // Handle databases - load collections if empty, toggle if has children
        else if (selectedNode.type === 'database') {
          if (selectedNode.children.length === 0) {
            dispatch({ payload: selectedNode.id, type: 'TOGGLE_EXPAND' });
            void loadCollections(
              selectedNode.connectionId!,
              selectedNode.databaseName!,
              selectedNode.id
            );
          } else {
            dispatch({ payload: selectedNode.id, type: 'TOGGLE_EXPAND' });
          }
        }
        // Handle collections - switch focus to panel
        else if (selectedNode.type === 'collection') {
          setActiveZone('panel');
        }
      }
    }

    if (key.name === 'n') {
      if (selectedNode?.type === 'connection') {
        openModal('create-database', { connectionAlias: selectedNode.name });
      } else if (selectedNode?.type === 'database') {
        openModal('create-collection', {
          connectionAlias: selectedNode.connectionId,
          databaseName: selectedNode.name,
        });
      } else if (!selectedNode) {
        openModal('add-connection');
      }
    }
  });

  const handleSearchChange = useCallback(
    (value: string) => {
      dispatch({ payload: value, type: 'SET_SEARCH' });
    },
    [dispatch]
  );

  const handleSearchClose = useCallback(() => {
    dispatch({ type: 'END_SEARCH' });
  }, [dispatch]);

  if (state.navigationTree.length === 0) {
    return (
      <box flexDirection="column" padding={1}>
        <text fg={colors.muted}>No connections</text>
        <text fg={colors.muted}>Press 'n' to add one</text>
      </box>
    );
  }

  return (
    <box flexDirection="column" flexGrow={1}>
      {state.isSearching && (
        <SearchInput
          value={state.searchQuery}
          onChange={handleSearchChange}
          onClose={handleSearchClose}
          placeholder="Filter..."
        />
      )}

      {!state.isSearching && state.searchQuery && (
        <box height={1} paddingLeft={1}>
          <text fg={colors.muted}>
            <span fg={colors.primary}>/</span>
            {state.searchQuery}
            <span fg={colors.dim}> (Esc to clear)</span>
          </text>
        </box>
      )}

      <scrollbox ref={scrollboxRef} flexGrow={1} scrollY>
        <box flexDirection="column">
          {filteredTree.length === 0 ? (
            <box padding={1}>
              <text fg={colors.muted}>No matches found</text>
            </box>
          ) : (
            filteredTree.map((node, index) => (
              <TreeNodeRenderer
                key={node.id}
                node={node}
                depth={0}
                selectedNodeId={state.selectedNodeId}
                isLast={index === filteredTree.length - 1}
                prefix=""
              />
            ))
          )}
        </box>
      </scrollbox>
    </box>
  );
}

interface TreeNodeRendererProps {
  node: TreeNodeData;
  depth: number;
  selectedNodeId: string | null;
  isLast?: boolean;
  prefix?: string;
}

function TreeNodeRenderer({
  node,
  depth,
  selectedNodeId,
  isLast = false,
  prefix = '',
}: TreeNodeRendererProps) {
  const isSelected = node.id === selectedNodeId;
  const isLeaf = node.children.length === 0 && node.type === 'collection';

  const childPrefix = depth > 0 ? prefix + (isLast ? '  ' : '│ ') : '';

  if (isLeaf) {
    return (
      <TreeLeaf node={node} depth={depth} isSelected={isSelected} isLast={isLast} prefix={prefix} />
    );
  }

  return (
    <TreeNode node={node} depth={depth} isSelected={isSelected} isLast={isLast} prefix={prefix}>
      {node.children.map((child, index) => (
        <TreeNodeRenderer
          key={child.id}
          node={child}
          depth={depth + 1}
          selectedNodeId={selectedNodeId}
          isLast={index === node.children.length - 1}
          prefix={childPrefix}
        />
      ))}
    </TreeNode>
  );
}

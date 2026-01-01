import { useFocus } from '../../context/focus-context.tsx';
import { useApp } from '../../context/app-context.tsx';
import { useModal } from '../../context/modal-context.tsx';
import { Sidebar } from './sidebar.tsx';
import { Panel } from './panel.tsx';
import { KeyboardHints } from './keyboard-hints.tsx';
import { ServerBrowser } from '../navigation/server-browser.tsx';
import { DetailPanel } from './detail-panel.tsx';
import type { TreeNodeData } from '../../types/index.ts';

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

function findNode(nodes: TreeNodeData[], id: string | null): TreeNodeData | null {
  if (!id) {
    return null;
  }
  for (const node of nodes) {
    if (node.id === id) {
      return node;
    }
    const found = findNode(node.children, id);
    if (found) {
      return found;
    }
  }
  return null;
}

function getPanelTitle(node: TreeNodeData): string {
  switch (node.type) {
    case 'connection':
      return `Connection: ${node.name}`;
    case 'database':
      return `Database: ${node.name}`;
    case 'collection':
      return `Collection: ${node.name}`;
    default:
      return node.name;
  }
}

function getBreadcrumb(nodes: TreeNodeData[], selectedId: string | null): string | null {
  if (!selectedId) return null;

  const node = findNode(nodes, selectedId);
  if (!node) return null;

  // Build path by traversing up through parents
  const path: string[] = [];
  let current: TreeNodeData | null = node;

  while (current) {
    // Only add parent nodes to breadcrumb, not the current selection
    if (current.id !== selectedId) {
      path.unshift(current.name);
    }
    current = current.parentId ? findNode(nodes, current.parentId) : null;
  }

  // Return null if at root level (connection selected or no parents)
  if (path.length === 0) return null;

  return path.join(' > ');
}

export function MainLayout() {
  const { activeZone } = useFocus();
  const { state } = useApp();
  const { modalStack } = useModal();

  const hasModal = modalStack.length > 0;
  const isSidebarFocused = !hasModal && activeZone === 'sidebar';
  const isPanelFocused = !hasModal && activeZone === 'panel';

  const selectedNode = findNode(state.navigationTree, state.selectedNodeId);
  const panelTitle = selectedNode ? getPanelTitle(selectedNode) : undefined;
  const selectionType = selectedNode?.type as 'connection' | 'database' | 'collection' | undefined;
  const hintsZone = hasModal ? 'modal' : activeZone;
  const breadcrumb = getBreadcrumb(state.navigationTree, state.selectedNodeId);

  // Get connection status for hints
  const connectionStatus =
    selectedNode?.type === 'connection'
      ? state.connections.find((c) => c.id === selectedNode.connectionId)?.status
      : undefined;

  const flatNodes = flattenTree(state.navigationTree);
  const itemCount = flatNodes.length;
  const selectedIndex = state.selectedNodeId
    ? flatNodes.findIndex((n) => n.id === state.selectedNodeId)
    : -1;

  return (
    <box flexDirection="column" flexGrow={1}>
      <box flexDirection="row" flexGrow={1}>
        <Sidebar
          focused={isSidebarFocused}
          itemCount={itemCount}
          selectedIndex={selectedIndex >= 0 ? selectedIndex : 0}
          breadcrumb={breadcrumb}
        >
          <ServerBrowser focused={isSidebarFocused} />
        </Sidebar>
        <Panel title={panelTitle} focused={isPanelFocused}>
          <DetailPanel focused={isPanelFocused} />
        </Panel>
      </box>
      <KeyboardHints
        activeZone={hintsZone}
        hasSelection={Boolean(selectedNode)}
        selectionType={selectionType}
        connectionStatus={connectionStatus}
      />
    </box>
  );
}

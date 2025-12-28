import { useEffect, useState } from 'react';
import { useKeyboard } from '@opentui/react';
import { AppProvider } from './context/app-context.tsx';
import { FocusProvider, useFocus } from './context/focus-context.tsx';
import { ModalProvider, useModal } from './context/modal-context.tsx';
import { MainLayout } from './components/layout/main-layout.tsx';
import { ModalRenderer } from './components/shared/modal-renderer.tsx';
import { loadConnections } from './services/persistence.ts';
import { parseConnectionString } from './utils/connection-string-parser.ts';
import { colors } from './theme/index.ts';
import type { Connection, TreeNodeData } from './types/index.ts';

function AppContent() {
  const { cycleZone, setActiveZone } = useFocus();
  const { closeModal, currentModal } = useModal();

  useKeyboard((key) => {
    const hasModal = Boolean(currentModal);

    if (key.name === 'tab' && !key.ctrl) {
      cycleZone(key.shift);
    }
    if (key.name === 'escape' && currentModal) {
      closeModal();
    }
    if (key.name === 'q' && key.ctrl) {
      process.exit(0);
    }

    if (!hasModal) {
      if (key.name === '1') {
        setActiveZone('sidebar');
      }
      if (key.name === '2') {
        setActiveZone('panel');
      }
    }
  });

  return (
    <box flexGrow={1}>
      <MainLayout />
      {currentModal && <ModalRenderer />}
    </box>
  );
}

interface InitialData {
  connections: Connection[];
  tree: TreeNodeData[];
}

export function App() {
  const [initialData, setInitialData] = useState<InitialData | null>(null);
  const [loadError, setLoadError] = useState<string>();

  useEffect(() => {
    loadConnections()
      .then((connections) => {
        // Parse each connection string
        const withParsed = connections.map((c) => {
          const result = parseConnectionString(c.url);
          return { ...c, parsed: result.parsed };
        });

        // Build initial tree
        const tree: TreeNodeData[] = withParsed.map((conn) => ({
          id: conn.id,
          type: 'connection' as const,
          name: conn.alias,
          parentId: null,
          isExpanded: false,
          children: [],
          connectionId: conn.id,
          status: 'disconnected' as const,
        }));

        setInitialData({ connections: withParsed, tree });
      })
      .catch((err) => {
        setLoadError(err instanceof Error ? err.message : 'Failed to load connections');
        setInitialData({ connections: [], tree: [] });
      });
  }, []);

  if (!initialData) {
    return (
      <box flexGrow={1} justifyContent="center" alignItems="center">
        <text fg={colors.muted}>Loading...</text>
      </box>
    );
  }

  if (loadError) {
    return (
      <box flexGrow={1} justifyContent="center" alignItems="center" flexDirection="column">
        <text fg={colors.statusError}>Failed to load connections</text>
        <text fg={colors.muted}>{loadError}</text>
      </box>
    );
  }

  return (
    <AppProvider initialConnections={initialData.connections} initialTree={initialData.tree}>
      <FocusProvider>
        <ModalProvider>
          <AppContent />
        </ModalProvider>
      </FocusProvider>
    </AppProvider>
  );
}

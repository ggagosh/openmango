import { useApp } from '../context/app-context.tsx';
import { mongoService, listCollections } from '../services/index.ts';
import type { TreeNodeData } from '../types/index.ts';

export function useConnection() {
  const { state, dispatch } = useApp();

  const connect = async (connectionId: string) => {
    const connection = state.connections.find((c) => c.id === connectionId);
    if (!connection) {
      dispatch({
        type: 'SET_STATUS',
        payload: { message: 'Connection not found', type: 'error' },
      });
      return;
    }

    dispatch({ type: 'CONNECT_START', payload: connectionId });

    try {
      await mongoService.connect(connectionId, connection.url);
      const databases = await mongoService.listDatabases(connectionId);
      const dbNames = databases.map((db) => db.name);

      dispatch({
        type: 'CONNECT_SUCCESS',
        payload: { id: connectionId, databases: dbNames },
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Connection failed';
      dispatch({
        type: 'CONNECT_ERROR',
        payload: { id: connectionId, error: message },
      });
    }
  };

  const disconnect = async (connectionId: string) => {
    try {
      await mongoService.disconnect(connectionId);
      dispatch({ type: 'DISCONNECT', payload: connectionId });
      dispatch({
        type: 'SET_STATUS',
        payload: { message: 'Disconnected', type: 'info' },
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Disconnect failed';
      dispatch({
        type: 'SET_STATUS',
        payload: { message, type: 'error' },
      });
    }
  };

  const loadCollections = async (connectionId: string, databaseName: string, nodeId: string) => {
    const client = mongoService.getClient(connectionId);
    if (!client) {
      dispatch({
        type: 'SET_STATUS',
        payload: { message: 'Not connected', type: 'error' },
      });
      return;
    }

    dispatch({ type: 'SET_LOADING', payload: true });

    try {
      const collections = await listCollections(client, databaseName);
      const collectionNodes: TreeNodeData[] = collections.map((collName) => ({
        id: `${connectionId}:${databaseName}:${collName}`,
        type: 'collection' as const,
        name: collName,
        parentId: nodeId,
        isExpanded: false,
        children: [],
        connectionId,
        databaseName,
        collectionName: collName,
      }));

      dispatch({
        type: 'UPDATE_NODE_CHILDREN',
        payload: { nodeId, children: collectionNodes },
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Failed to load collections';
      dispatch({
        type: 'SET_STATUS',
        payload: { message, type: 'error' },
      });
    } finally {
      dispatch({ type: 'SET_LOADING', payload: false });
    }
  };

  const refreshConnection = async (connectionId: string) => {
    const connection = state.connections.find((c) => c.id === connectionId);
    if (!connection || connection.status !== 'connected') return;

    try {
      const databases = await mongoService.listDatabases(connectionId);
      const dbNames = databases.map((db) => db.name);
      dispatch({
        type: 'CONNECT_SUCCESS',
        payload: { id: connectionId, databases: dbNames },
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Refresh failed';
      dispatch({
        type: 'SET_STATUS',
        payload: { message, type: 'error' },
      });
    }
  };

  return {
    connect,
    disconnect,
    loadCollections,
    refreshConnection,
    isConnected: (id: string) => mongoService.isConnected(id),
  };
}

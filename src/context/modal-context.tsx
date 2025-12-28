import { type ReactNode, createContext, useCallback, useContext, useState } from 'react';
import type { ModalType } from '../types/index.ts';

interface ModalEntry {
  type: ModalType;
  props?: Record<string, unknown>;
}

interface ModalContextValue {
  modalStack: ModalEntry[];
  openModal: (type: ModalType, props?: Record<string, unknown>) => void;
  closeModal: () => void;
  closeAllModals: () => void;
  currentModal: ModalEntry | null;
}

const ModalContext = createContext<ModalContextValue | null>(null);

export function ModalProvider({ children }: { children: ReactNode }) {
  const [modalStack, setModalStack] = useState<ModalEntry[]>([]);

  const openModal = useCallback((type: ModalType, props?: Record<string, unknown>) => {
    setModalStack((stack) => [...stack, { props, type }]);
  }, []);

  const closeModal = useCallback(() => {
    setModalStack((stack) => stack.slice(0, -1));
  }, []);

  const closeAllModals = useCallback(() => {
    setModalStack([]);
  }, []);

  const currentModal: ModalEntry | null =
    modalStack.length > 0 ? modalStack[modalStack.length - 1]! : null;

  return (
    <ModalContext.Provider
      value={{ closeAllModals, closeModal, currentModal, modalStack, openModal }}
    >
      {children}
    </ModalContext.Provider>
  );
}

export function useModal() {
  const context = useContext(ModalContext);
  if (!context) {
    throw new Error('useModal must be used within ModalProvider');
  }
  return context;
}

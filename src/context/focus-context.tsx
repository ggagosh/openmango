import { type ReactNode, createContext, useCallback, useContext, useState } from 'react';
import type { FocusZone } from '../types/index.ts';

interface FocusContextValue {
  activeZone: FocusZone;
  setActiveZone: (zone: FocusZone) => void;
  cycleZone: (reverse?: boolean) => void;
}

const FocusContext = createContext<FocusContextValue | null>(null);

export function FocusProvider({ children }: { children: ReactNode }) {
  const [activeZone, setActiveZone] = useState<FocusZone>('sidebar');

  const cycleZone = useCallback((reverse = false) => {
    setActiveZone((current) => {
      if (current === 'modal') {
        return current;
      } // Don't cycle when modal is open
      if (reverse) {
        return current === 'sidebar' ? 'panel' : 'sidebar';
      }
      return current === 'sidebar' ? 'panel' : 'sidebar';
    });
  }, []);

  return (
    <FocusContext.Provider value={{ activeZone, cycleZone, setActiveZone }}>
      {children}
    </FocusContext.Provider>
  );
}

export function useFocus() {
  const context = useContext(FocusContext);
  if (!context) {
    throw new Error('useFocus must be used within FocusProvider');
  }
  return context;
}

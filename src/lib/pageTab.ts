// Internal pages render for "their" tab: the active tab normally, or the pane's tab in split view.
import { createContext, useContext } from 'react';
import type { Tab } from '../types/browser';
import { useActiveTab } from './store';

export const PageTabContext = createContext<Tab | undefined>(undefined);

export function usePageTab(): Tab | undefined {
  const pane = useContext(PageTabContext);
  const active = useActiveTab();
  return pane ?? active;
}

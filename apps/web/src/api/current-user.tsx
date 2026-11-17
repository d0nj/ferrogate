import { createContext, useContext } from 'react';
import type { ReactNode } from 'react';

import type { CurrentUser } from './auth';

const CurrentUserContext = createContext<CurrentUser | null>(null);

interface CurrentUserProviderProps {
  user: CurrentUser | null;
  children: ReactNode;
}

export function CurrentUserProvider({ user, children }: CurrentUserProviderProps) {
  return <CurrentUserContext.Provider value={user}>{children}</CurrentUserContext.Provider>;
}

// eslint-disable-next-line only-export-components
export function useCurrentUser(): CurrentUser | null {
  return useContext(CurrentUserContext);
}

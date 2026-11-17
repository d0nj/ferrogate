import { useMemo, useState } from 'react'
import type { ReactNode } from 'react'

import { PageActionContext } from './page-action-context'

export function PageActionProvider({ children }: { children: ReactNode }) {
  const [actions, setActions] = useState<ReactNode>(null)
  const [detail, setDetail] = useState<ReactNode>(null)
  const value = useMemo(() => ({ actions, detail, setActions, setDetail }), [actions, detail])
  return <PageActionContext.Provider value={value}>{children}</PageActionContext.Provider>
}

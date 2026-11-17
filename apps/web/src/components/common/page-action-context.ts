import { createContext } from 'react'
import type { ReactNode } from 'react'

export interface PageChrome {
  actions: ReactNode
  detail: ReactNode
  setActions: (next: ReactNode) => void
  setDetail: (next: ReactNode) => void
}

export const PageActionContext = createContext<PageChrome | null>(null)

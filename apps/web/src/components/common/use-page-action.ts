import { useContext, useEffect, useMemo } from 'react'
import type { ReactNode } from 'react'

import { PageActionContext } from './page-action-context'

/**
 * Registers the calling page's primary action with the shell header.
 * The factory runs again only when a dependency changes, so the node
 * identity stays stable across unrelated re-renders.
 */
export function usePageAction(render: () => ReactNode, deps: readonly unknown[]) {
  const setActions = useContext(PageActionContext)?.setActions
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const node = useMemo(render, deps)

  useEffect(() => {
    if (!setActions) return
    setActions(node)
    return () => setActions(null)
  }, [node, setActions])
}

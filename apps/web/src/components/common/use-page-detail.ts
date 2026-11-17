import { useContext, useEffect, useMemo } from 'react'
import type { ReactNode } from 'react'

import { PageActionContext } from './page-action-context'

/**
 * Registers a supplementary line under the page title — record counts, the
 * active tab's scope — so a page never has to invent its own header bar.
 */
export function usePageDetail(render: () => ReactNode, deps: readonly unknown[]) {
  const setDetail = useContext(PageActionContext)?.setDetail
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const node = useMemo(render, deps)

  useEffect(() => {
    if (!setDetail) return
    setDetail(node)
    return () => setDetail(null)
  }, [node, setDetail])
}

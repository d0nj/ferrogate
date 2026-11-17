import { useState } from 'react'

import { ConfirmDialog } from '@/components/common/ConfirmDialog'

import type { ConfirmRequest } from '@/components/common/ConfirmDialog'

/**
 * Pairs a {@link ConfirmRequest} with the dialog that guards it.
 *
 * Returns the request to hand to `setConfirm` when the user asks to delete, and
 * the element to render once. Keeping the state here means a page never repeats
 * the open/close plumbing for each entity it can destroy.
 */
export function useConfirmDialog() {
  const [request, setRequest] = useState<ConfirmRequest | null>(null)

  const dialog = (
    <ConfirmDialog
      isOpen={request !== null}
      onOpenChange={(isOpen) => {
        if (!isOpen) setRequest(null)
      }}
      title={request?.title ?? ''}
      description={request?.description ?? ''}
      actionLabel={request?.actionLabel ?? 'Delete'}
      onConfirm={async () => {
        await request?.onConfirm()
      }}
    />
  )

  return { ask: setRequest, dialog }
}

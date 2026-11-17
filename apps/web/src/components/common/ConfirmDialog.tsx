import { useState } from 'react'

import { AlertDialog } from '@astryxdesign/core/AlertDialog'

/** What a destructive action needs to ask before it runs. */
export interface ConfirmRequest {
  title: string
  description: string
  actionLabel: string
  onConfirm: () => void | Promise<void>
}

interface ConfirmDialogProps extends ConfirmRequest {
  isOpen: boolean
  onOpenChange: (isOpen: boolean) => void
}

/**
 * Asks before an irreversible action runs.
 *
 * `AlertDialog` does not close on its own, so the dialog stays open (with the
 * action spinning) until the caller's request settles — a failed delete then
 * reports in place instead of flashing shut.
 */
export function ConfirmDialog({
  isOpen,
  onOpenChange,
  title,
  description,
  actionLabel,
  onConfirm,
}: ConfirmDialogProps) {
  const [isPending, setIsPending] = useState(false)

  async function run() {
    setIsPending(true)
    try {
      await onConfirm()
      onOpenChange(false)
    } finally {
      setIsPending(false)
    }
  }

  return (
    <AlertDialog
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      title={title}
      description={description}
      actionLabel={actionLabel}
      isActionLoading={isPending}
      onAction={() => {
        void run()
      }}
    />
  )
}

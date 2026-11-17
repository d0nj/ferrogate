import { useContext } from 'react'

import { Heading } from '@astryxdesign/core/Heading'
import { Text } from '@astryxdesign/core/Text'
import { HStack, StackItem } from '@astryxdesign/core/Stack'

import { PageActionContext } from '@/components/common/page-action-context'

import type { NavEntry } from '@/App'

export function ControlPlaneHeader({ page }: { page: NavEntry | undefined }) {
  const chrome = useContext(PageActionContext)
  const actions = chrome?.actions ?? null
  const detail = chrome?.detail ?? null

  if (!page) return null

  return (
    <HStack gap={3} vAlign="center" wrap="wrap">
      <StackItem size="fill">
        <HStack gap={2} vAlign="end">
          <Heading level={1}>{page.label}</Heading>
          {detail ? <Text color="secondary">{detail}</Text> : null}
        </HStack>
      </StackItem>
      {actions ? <HStack gap={2} vAlign="center">{actions}</HStack> : null}
    </HStack>
  )
}

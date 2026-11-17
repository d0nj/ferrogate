import { useEffect, useMemo, useState } from 'react'

import { Banner } from '@astryxdesign/core/Banner'
import { Card } from '@astryxdesign/core/Card'
import { Selector } from '@astryxdesign/core/Selector'
import { HStack, VStack } from '@astryxdesign/core/Stack'
import { Heading, Text } from '@astryxdesign/core/Text'
import { TextArea } from '@astryxdesign/core/TextArea'

import { api } from '@/api/client'
import type { PromptRenderResponse, PromptSummaryDto, PromptVersionDto } from '@/api/types'

export interface TemplateSelection {
  slug: string
  version: number | null
}

export interface TemplatePickerValue {
  selection: TemplateSelection | null
  variables: Record<string, string>
}

interface PromptTemplatePickerProps {
  value: TemplatePickerValue
  onChange: (next: TemplatePickerValue) => void
  onRendered?: (rendered: PromptRenderResponse | null) => void
}

const NO_TEMPLATE = ''

function defaultVariables(template: PromptVersionDto | undefined): Record<string, string> {
  const values: Record<string, string> = {}
  for (const variable of template?.variables ?? []) {
    values[variable.name] = variable.default ?? ''
  }
  return values
}

export function PromptTemplatePicker({ value, onChange, onRendered }: PromptTemplatePickerProps) {
  const { selection, variables } = value
  const selectedSlug = selection?.slug ?? null
  const [summaries, setSummaries] = useState<PromptSummaryDto[]>([])
  const [versions, setVersions] = useState<PromptVersionDto[]>([])
  const [error, setError] = useState<string | null>(null)
  const [rendered, setRendered] = useState<PromptRenderResponse | null>(null)

  useEffect(() => {
    let cancelled = false
    api
      .listPrompts()
      .then((items) => {
        if (!cancelled) setSummaries(items)
      })
      .catch((cause: unknown) => {
        if (!cancelled) setError(cause instanceof Error ? cause.message : 'Failed to load templates')
      })
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    if (selectedSlug === null) return
    let cancelled = false
    api
      .listPromptVersions(selectedSlug)
      .then((response) => {
        if (!cancelled) setVersions(response.versions)
      })
      .catch((cause: unknown) => {
        if (!cancelled) setError(cause instanceof Error ? cause.message : 'Failed to load versions')
      })
    return () => {
      cancelled = true
    }
  }, [selectedSlug])

  const activeTemplate = useMemo(() => {
    if (!selection) return undefined
    return versions.find((item) => item.version === selection.version) ?? versions[0]
  }, [versions, selection])

  const renderKey = JSON.stringify(selection ? [selection.slug, selection.version, variables] : null)

  useEffect(() => {
    const request = JSON.parse(renderKey) as [string, number | null, Record<string, string>] | null
    if (request === null) return
    const [requestSlug, requestVersion, requestVariables] = request
    let cancelled = false
    api
      .renderPrompt(requestSlug, requestVariables, requestVersion ?? undefined)
      .then((response) => {
        if (cancelled) return
        setRendered(response)
        onRendered?.(response)
      })
      .catch((cause: unknown) => {
        if (!cancelled) setError(cause instanceof Error ? cause.message : 'Render failed')
      })
    return () => {
      cancelled = true
    }
    // `renderKey` collapses slug/version/variables so the preview refetches once per edit.
  }, [renderKey, onRendered])

  const chooseTemplate = (slug: string) => {
    const summary = summaries.find((item) => item.slug === slug)
    setError(null)
    if (!summary) {
      setVersions([])
      setRendered(null)
      onRendered?.(null)
      onChange({ selection: null, variables: {} })
      return
    }
    const template = versions.find((item) => item.slug === slug)
    onChange({
      selection: { slug, version: summary.latest_version },
      variables: defaultVariables(template),
    })
  }

  return (
    <Card padding={4}>
      <VStack gap={3}>
        <HStack hAlign="between" vAlign="center">
          <Heading level={3}>Template</Heading>
          <Text type="supporting">{selection ? selection.slug : 'free-form'}</Text>
        </HStack>

        <Selector
          label="Prompt template"
          size="sm"
          options={[NO_TEMPLATE, ...summaries.map((item) => item.slug)]}
          value={selection?.slug ?? NO_TEMPLATE}
          onChange={chooseTemplate}
          placeholder="Free-form messages"
          width="100%"
        />

        {selection ? (
          <Selector
            label="Version"
            size="sm"
            options={versions.map((item) => String(item.version))}
            value={String(selection.version ?? '')}
            onChange={(raw) => {
              const version = Number(raw)
              if (!selection || Number.isNaN(version)) return
              const template = versions.find((item) => item.version === version)
              onChange({
                selection: { slug: selection.slug, version },
                variables: defaultVariables(template),
              })
            }}
            width="100%"
          />
        ) : null}

        {activeTemplate?.variables.map((variable) => (
          <TextArea
            key={variable.name}
            label={variable.name}
            value={variables[variable.name] ?? ''}
            onChange={(next) => onChange({ selection, variables: { ...variables, [variable.name]: next } })}
            placeholder={variable.default ?? `Value for ${variable.name}`}
            rows={2}
          />
        ))}

        {rendered ? (
          <VStack gap={1}>
            <Text type="supporting">Rendered prompt</Text>
            <Text type="code">{rendered.user}</Text>
            {rendered.system ? (
              <VStack gap={1}>
                <Text type="supporting">System</Text>
                <Text type="code">{rendered.system}</Text>
              </VStack>
            ) : null}
          </VStack>
        ) : null}

        {rendered && rendered.missing_variables.length > 0 ? (
          <Banner
            status="warning"
            title="Missing variables"
            description={rendered.missing_variables.join(', ')}
          />
        ) : null}

        {error ? <Banner status="error" title={error} /> : null}
      </VStack>
    </Card>
  )
}

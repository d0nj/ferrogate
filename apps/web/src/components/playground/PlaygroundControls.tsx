import { Badge } from '@astryxdesign/core/Badge'
import { Button } from '@astryxdesign/core/Button'
import { Section } from '@astryxdesign/core/Section'
import { NumberInput } from '@astryxdesign/core/NumberInput'
import { Slider } from '@astryxdesign/core/Slider'
import { HStack, StackItem, VStack } from '@astryxdesign/core/Stack'
import { Heading, Text } from '@astryxdesign/core/Text'
import { TextArea } from '@astryxdesign/core/TextArea'
import { TextInput } from '@astryxdesign/core/TextInput'

import type { StreamStats } from '@/api/types'
import { formatMs } from '@/utils/format'

export interface PlaygroundSettings {
  apiKey: string
  model: string
  systemPrompt: string
  temperature: number
  maxTokens: number
}

interface PlaygroundControlsProps {
  settings: PlaygroundSettings
  onChange: (next: Partial<PlaygroundSettings>) => void
  isStreaming: boolean
  stats: StreamStats | null
  isTemplateSelected: boolean
  onUseSavedKey: () => void
  isMintingKey: boolean
}

interface ReadoutProps {
  label: string
  value: string
}

function Readout({ label, value }: ReadoutProps) {
  return (
    <VStack gap={0.5} width={72}>
      <Text type="supporting">{label}</Text>
      <Text weight="semibold" hasTabularNumbers>
        {value}
      </Text>
    </VStack>
  )
}

export function PlaygroundControls({
  settings,
  onChange,
  isStreaming,
  stats,
  isTemplateSelected,
  onUseSavedKey,
  isMintingKey,
}: PlaygroundControlsProps) {
  return (
    <VStack gap={5}>
      <Section padding={0} dividers={['bottom']}>
        <VStack gap={3}>
          <Heading level={3}>Connection</Heading>
          <HStack gap={2} align="end" width="100%">
            <StackItem size="fill">
              <TextInput
                label="Virtual key"
                value={settings.apiKey}
                onChange={(value) => onChange({ apiKey: value })}
                placeholder="sk-fg-…"
                width="100%"
              />
            </StackItem>
            <StackItem>
              <Button
                label={isMintingKey ? "Creating…" : "Get key"}
                variant="secondary"
                size="sm"
                isDisabled={isMintingKey}
                onClick={onUseSavedKey}
              />
            </StackItem>
          </HStack>
        </VStack>
      </Section>

      <Section padding={0} dividers={['bottom']}>
        <VStack gap={3}>
          <Heading level={3}>Sampling</Heading>
          <TextArea
            label="System prompt"
            value={settings.systemPrompt}
            onChange={(value) => onChange({ systemPrompt: value })}
            placeholder="You are a concise assistant."
            description={
              isTemplateSelected
                ? 'Ignored while a template is selected; the template supplies both messages.'
                : undefined
            }
            isDisabled={isTemplateSelected}
            rows={3}
            size="sm"
          />
          <Slider
            label="Temperature"
            value={settings.temperature}
            onChange={(value: number) => onChange({ temperature: value })}
            min={0}
            max={2}
            step={0.1}
            valueDisplay="text"
            formatValue={(value) => value.toFixed(1)}
          />
          <NumberInput
            label="Max tokens"
            value={settings.maxTokens}
            onChange={(value) => onChange({ maxTokens: value })}
            min={1}
            max={32_768}
            step={256}
            size="sm"
            units="tokens"
            hasNumberSteppers
          />
        </VStack>
      </Section>

      <Section padding={0} dividers={['bottom']}>
        <VStack gap={3}>
          <HStack hAlign="between" vAlign="center">
            <Heading level={3}>Telemetry</Heading>
            <Badge variant={isStreaming ? 'blue' : 'neutral'} label={isStreaming ? 'Streaming' : 'Idle'} />
          </HStack>
          <HStack gap={4} wrap="wrap">
            <Readout label="TTFT" value={formatMs(stats?.ttftMs)} />
            <Readout
              label="Tokens/sec"
              value={stats ? stats.tokensPerSec.toFixed(1) : '—'}
            />
            <Readout label="Output tokens" value={stats ? String(stats.outputTokens) : '—'} />
            <Readout label="Total" value={formatMs(stats?.totalMs)} />
          </HStack>
        </VStack>
      </Section>
    </VStack>
  )
}

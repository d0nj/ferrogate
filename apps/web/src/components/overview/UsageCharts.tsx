import { useMemo, useState } from 'react'

import { Card } from '@astryxdesign/core/Card'
import { Section } from '@astryxdesign/core/Section'
import { SegmentedControl, SegmentedControlItem } from '@astryxdesign/core/SegmentedControl'
import { HStack, VStack } from '@astryxdesign/core/Stack'
import { Heading, Text } from '@astryxdesign/core/Text'
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'

import type { TimeseriesBucket } from '@/api/types'
import { formatNumber, formatUsd } from '@/utils/format'

type Metric = 'tokens' | 'spend'

const SERIES_COLORS = [
  'var(--color-data-categorical-orange)',
  'var(--color-data-categorical-cyan)',
  'var(--color-data-categorical-green)',
  'var(--color-data-categorical-purple)',
  'var(--color-data-categorical-blue)',
  'var(--color-data-categorical-pink)',
  'var(--color-data-categorical-red)',
  'var(--color-data-categorical-teal)',
]

const MAX_SERIES = SERIES_COLORS.length

interface ChartPoint {
  label: string
  [series: string]: string | number
}

interface ChartModel {
  points: ChartPoint[]
  series: string[]
}

function seriesKey(model: string, provider: string): string {
  return `${model} · ${provider}`
}

function buildModel(buckets: TimeseriesBucket[], metric: Metric): ChartModel {
  const totals = new Map<string, number>()
  for (const bucket of buckets) {
    for (const entry of bucket.by_model) {
      const key = seriesKey(entry.model, entry.provider)
      const amount = metric === 'tokens' ? entry.tokens : entry.spend_usd
      totals.set(key, (totals.get(key) ?? 0) + amount)
    }
  }

  const series = [...totals.entries()]
    .sort((a, b) => b[1] - a[1])
    .slice(0, MAX_SERIES)
    .map(([key]) => key)

  const points = buckets.map((bucket) => {
    const point: ChartPoint = { label: bucket.label }
    for (const key of series) {
      point[key] = 0
    }
    for (const entry of bucket.by_model) {
      const key = seriesKey(entry.model, entry.provider)
      if (series.includes(key)) {
        const amount = metric === 'tokens' ? entry.tokens : entry.spend_usd
        point[key] = (point[key] as number) + amount
      }
    }
    return point
  })

  return { points, series }
}

function formatAxis(value: number, metric: Metric): string {
  if (metric === 'spend') return `$${value.toFixed(value < 1 ? 2 : 0)}`
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`
  return String(value)
}

interface ChartTooltipProps {
  active?: boolean
  label?: string | number
  payload?: Array<{ dataKey?: string | number; value?: number; color?: string }>
  metric: Metric
}

function ChartTooltip({ active, label, payload = [], metric }: ChartTooltipProps) {
  if (!active || payload.length === 0) return null
  const rows = [...payload].sort((a, b) => (b.value ?? 0) - (a.value ?? 0))

  return (
    <Card padding={3} elevation="med">
      <VStack gap={1.5}>
        <Text weight="semibold">{String(label ?? '')}</Text>
        {rows.map((row) => (
          <HStack key={String(row.dataKey)} gap={2} vAlign="center" hAlign="between">
            <HStack gap={1.5} vAlign="center">
              <span
                aria-hidden="true"
                style={{
                  width: 8,
                  height: 8,
                  borderRadius: 'var(--radius-full)',
                  backgroundColor: row.color,
                }}
              />
              <Text type="supporting" maxLines={1}>
                {String(row.dataKey)}
              </Text>
            </HStack>
            <Text type="supporting" hasTabularNumbers>
              {metric === 'spend' ? formatUsd(row.value ?? 0) : formatNumber(row.value ?? 0)}
            </Text>
          </HStack>
        ))}
      </VStack>
    </Card>
  )
}

interface UsageChartsProps {
  buckets: TimeseriesBucket[]
  isLoading: boolean
}

export function UsageCharts({ buckets, isLoading }: UsageChartsProps) {
  const [metric, setMetric] = useState<Metric>('tokens')
  const { points, series } = useMemo(() => buildModel(buckets, metric), [buckets, metric])

  return (
    <Section paddingInline={0} paddingBlock={4} dividers={['bottom']}>
      <VStack gap={4}>
        <HStack hAlign="between" vAlign="end" gap={3} wrap="wrap">
          <VStack gap={1}>
            <Text type="supporting" size="2xs" weight="medium" textWrap="nowrap">
              USAGE
            </Text>
            <Heading level={2}>Tokens &amp; spend by model</Heading>
            <Text type="supporting">
              Stacked by model · provider, top {MAX_SERIES} series in the window
            </Text>
          </VStack>
          <SegmentedControl
            label="Usage metric"
            size="sm"
            value={metric}
            onChange={(next) => setMetric(next as Metric)}
          >
            <SegmentedControlItem value="tokens" label="Tokens" />
            <SegmentedControlItem value="spend" label="Spend ($)" />
          </SegmentedControl>
        </HStack>

        {series.length === 0 ? (
          <Text color="secondary">{isLoading ? 'Loading series…' : 'No traffic in this window'}</Text>
        ) : (
          <VStack width="100%" height={288}>
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={points} margin={{ top: 8, right: 8, bottom: 0, left: 0 }}>
                <CartesianGrid
                  stroke="var(--color-border)"
                  strokeDasharray="3 3"
                  vertical={false}
                />
                <XAxis
                  dataKey="label"
                  tickLine={false}
                  axisLine={false}
                  minTickGap={24}
                  tick={{ fill: 'var(--color-text-secondary)', fontSize: 11 }}
                />
                <YAxis
                  tickLine={false}
                  axisLine={false}
                  width={56}
                  tickFormatter={(value: number) => formatAxis(value, metric)}
                  tick={{ fill: 'var(--color-text-secondary)', fontSize: 11 }}
                />
                <Tooltip
                  cursor={{ stroke: 'var(--color-border-emphasized)' }}
                  content={<ChartTooltip metric={metric} />}
                />
                {series.map((key, index) => (
                  <Area
                    key={key}
                    type="monotone"
                    dataKey={key}
                    stackId="usage"
                    stroke={SERIES_COLORS[index % SERIES_COLORS.length]}
                    fill={SERIES_COLORS[index % SERIES_COLORS.length]}
                    fillOpacity={0.28}
                    strokeWidth={1.5}
                    isAnimationActive={false}
                  />
                ))}
              </AreaChart>
            </ResponsiveContainer>
          </VStack>
        )}
      </VStack>
    </Section>
  )
}

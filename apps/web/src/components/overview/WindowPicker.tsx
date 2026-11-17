import { SegmentedControl, SegmentedControlItem } from '@astryxdesign/core/SegmentedControl'

import type { OverviewWindow } from '@/api/types'

const WINDOWS: Array<{ value: OverviewWindow; label: string }> = [
  { value: '1h', label: '1h' },
  { value: '24h', label: '24h' },
  { value: '7d', label: '7d' },
]

interface WindowPickerProps {
  value: OverviewWindow
  onChange: (window: OverviewWindow) => void
}

export function WindowPicker({ value, onChange }: WindowPickerProps) {
  return (
    <SegmentedControl
      label="Time window"
      size="sm"
      value={value}
      onChange={(next) => onChange(next as OverviewWindow)}
    >
      {WINDOWS.map((option) => (
        <SegmentedControlItem key={option.value} value={option.value} label={option.label} />
      ))}
    </SegmentedControl>
  )
}

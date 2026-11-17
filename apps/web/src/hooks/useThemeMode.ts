import { useCallback, useEffect, useState } from 'react'

export type ThemeMode = 'light' | 'dark' | 'system'

const STORAGE_KEY = 'ferrogate.theme-mode'

/** Modes in the order the toggle cycles through them. */
export const THEME_MODES: readonly ThemeMode[] = ['dark', 'light', 'system']

function isMode(value: unknown): value is ThemeMode {
  return value === 'light' || value === 'dark' || value === 'system'
}

function storedMode(): ThemeMode {
  if (typeof window === 'undefined') return 'dark'
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY)
    return isMode(raw) ? raw : 'dark'
  } catch {
    // Private-mode or blocked storage: fall back without persisting.
    return 'dark'
  }
}

/**
 * Colour mode with persistence.
 *
 * The console has been dark-first since it shipped, so an OS following
 * `system` default would flip existing installs to light on first load; the
 * default stays `dark` and `system` is opt-in.
 */
export function useThemeMode() {
  const [mode, setMode] = useState<ThemeMode>(storedMode)

  useEffect(() => {
    try {
      window.localStorage.setItem(STORAGE_KEY, mode)
    } catch {
      // Persistence is best-effort; the session still needs the value.
    }
  }, [mode])

  const cycle = useCallback(() => {
    setMode((current) => {
      const index = THEME_MODES.indexOf(current)
      return THEME_MODES[(index + 1) % THEME_MODES.length]
    })
  }, [])

  return { mode, setMode, cycle }
}

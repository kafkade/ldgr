'use client';

import {
  createContext,
  useContext,
  useState,
  useEffect,
  useCallback,
  type ReactNode,
} from 'react';

type ThemePreference = 'light' | 'dark' | 'system';
type ResolvedTheme = 'light' | 'dark';

interface ThemeContextValue {
  /** User preference (may be 'system'). */
  preference: ThemePreference;
  /** Effective theme applied to the DOM. */
  theme: ResolvedTheme;
  setPreference: (pref: ThemePreference) => void;
  toggleTheme: () => void;
}

const ThemeContext = createContext<ThemeContextValue>({
  preference: 'system',
  theme: 'light',
  setPreference: () => {},
  toggleTheme: () => {},
});

export function useTheme() {
  return useContext(ThemeContext);
}

function resolveTheme(pref: ThemePreference, systemDark: boolean): ResolvedTheme {
  if (pref === 'system') return systemDark ? 'dark' : 'light';
  return pref;
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [preference, setPreferenceState] = useState<ThemePreference>('system');
  const [systemDark, setSystemDark] = useState(false);

  // Read stored preference + detect system scheme on mount
  useEffect(() => {
    const stored = localStorage.getItem('ldgr-theme') as ThemePreference | null;
    if (stored === 'light' || stored === 'dark' || stored === 'system') {
      setPreferenceState(stored);
    }
    setSystemDark(window.matchMedia('(prefers-color-scheme: dark)').matches);
  }, []);

  // Listen for system theme changes
  useEffect(() => {
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const handler = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  }, []);

  const resolved = resolveTheme(preference, systemDark);

  // Apply to DOM
  useEffect(() => {
    document.documentElement.classList.toggle('dark', resolved === 'dark');
  }, [resolved]);

  const setPreference = useCallback((pref: ThemePreference) => {
    setPreferenceState(pref);
    localStorage.setItem('ldgr-theme', pref);
  }, []);

  const toggleTheme = useCallback(() => {
    setPreferenceState((prev) => {
      const next = resolveTheme(prev, systemDark) === 'light' ? 'dark' : 'light';
      localStorage.setItem('ldgr-theme', next);
      return next;
    });
  }, [systemDark]);

  return (
    <ThemeContext.Provider value={{ preference, theme: resolved, setPreference, toggleTheme }}>
      {children}
    </ThemeContext.Provider>
  );
}

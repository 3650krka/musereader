import { createContext, useContext } from 'react';

export type Lang = 'zh' | 'en';

export const LANG_STORAGE_KEY = 'musereader-lang';
const LEGACY_LANG_STORAGE_KEY = 'musetranslate-lang';

export function detectInitialLang(): Lang {
  const stored =
    localStorage.getItem(LANG_STORAGE_KEY) ?? localStorage.getItem(LEGACY_LANG_STORAGE_KEY);
  if (stored === 'zh' || stored === 'en') return stored;
  return navigator.language.toLowerCase().startsWith('zh') ? 'zh' : 'en';
}

export interface LangContextValue {
  lang: Lang;
  setLang: (lang: Lang) => void;
}

export const LangContext = createContext<LangContextValue>({
  lang: 'zh',
  setLang: () => undefined,
});

export function useLang(): LangContextValue {
  return useContext(LangContext);
}

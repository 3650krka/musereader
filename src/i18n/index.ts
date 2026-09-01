import en from './en';
import zh, { type Messages } from './zh';
import { useLang } from './context';

const DICTS: Record<'zh' | 'en', Messages> = { zh, en };

export function getMessages(lang: 'zh' | 'en'): Messages {
  return DICTS[lang];
}

/** 组件内取文案：`const t = useT(); t.library.title` */
export function useT(): Messages {
  const { lang } = useLang();
  return DICTS[lang];
}

export type { Messages };

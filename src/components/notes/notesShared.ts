import type { Book, ReaderNote, ReaderState } from '@/types';
import type { Lang } from '@/i18n/context';

export type NotesItem = ReaderNote & {
  source: string;
  kind: 'anno' | 'bmk';
};

export type NotesFilter = 'all' | 'anno' | 'bmk';

export interface NotesSummary {
  items: NotesItem[];
  latestItem: NotesItem | null;
  bookmarkCount: number;
  recentItems: NotesItem[];
}

export function buildNotesSummary(
  books: Book[],
  readerStates: ReaderState[],
  lang: Lang,
  filter: NotesFilter = 'all',
): NotesSummary {
  const fallback = lang === 'zh' ? '未知书籍' : 'Unknown book';
  const titleOf = (bookId: string) => books.find((book) => book.id === bookId)?.title || fallback;

  const annoItems: NotesItem[] = readerStates.flatMap((state) =>
    state.notes.map((note) => ({ ...note, kind: 'anno' as const, source: titleOf(note.bookId) })),
  );
  const bmkItems: NotesItem[] = readerStates.flatMap((state) =>
    state.bookmarks.map((b) => ({
      ...b, kind: 'bmk' as const, note: b.quote, quote: '', source: titleOf(b.bookId),
    })),
  );

  const merged = [...annoItems, ...bmkItems]
    .sort((a, b) => new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime());
  const filtered = filter === 'all' ? merged : merged.filter((i) => i.kind === filter);

  return {
    latestItem: merged[0] ?? null,
    bookmarkCount: bmkItems.length,
    recentItems: filtered.slice(0, 3),
    items: filtered,
  };
}

import type { Book, RuntimeNotice, TranslationTask } from '@/types';

export interface WorkbenchConfig {
  articleType: string;
  concurrent: boolean;
  maxWorkers: number;
}

export interface SelectedImportFile {
  name: string;
  path: string;
}

export interface LibraryShelfViewModel {
  currentBook: Book | null;
  continueShelf: Book[];
  catalogBooks: Book[];
}

export function sameNoticeList(previous: RuntimeNotice[], next: RuntimeNotice[]) {
  if (previous === next) return true;
  if (previous.length !== next.length) return false;

  return previous.every((notice, index) => {
    const candidate = next[index];
    return (
      notice.timestamp === candidate.timestamp &&
      notice.scope === candidate.scope &&
      notice.taskId === candidate.taskId &&
      notice.code === candidate.code &&
      notice.message === candidate.message &&
      notice.detail === candidate.detail
    );
  });
}

const SUPPORTED_IMPORT_EXTENSIONS = ['.pdf', '.epub', '.docx', '.md', '.markdown', '.txt'];

export function isSupportedImportFilename(filename: string) {
  const normalized = filename.toLowerCase();
  return SUPPORTED_IMPORT_EXTENSIONS.some((ext) => normalized.endsWith(ext));
}

export function filterBooksByQuery(books: Book[], searchQuery: string) {
  const normalizedQuery = searchQuery.trim().toLowerCase();
  if (!normalizedQuery) return books;

  return books.filter((book) => {
    const haystack = [book.title, book.author, book.category, book.level].join(' ').toLowerCase();
    return haystack.includes(normalizedQuery);
  });
}

export function buildPendingTaskDraft(
  taskId: string,
  file: SelectedImportFile,
  config: WorkbenchConfig,
  createdAt: string,
  message: string,
): TranslationTask {
  return {
    id: taskId,
    filename: file.name,
    pdfPath: file.path,
    status: 'pending',
    phase: 'imported',
    progress: 0,
    message,
    articleType: config.articleType,
    concurrent: config.concurrent,
    maxWorkers: config.maxWorkers,
    createdAt,
    updatedAt: createdAt,
    artifactPaths: {},
    totalChunks: 0,
    translatedChunks: 0,
    retryCount: 0,
    lastError: null,
  };
}

export type LibrarySortKey = 'recent' | 'title' | 'progress';

export function sortLibraryBooks(books: Book[], sort: LibrarySortKey): Book[] {
  if (sort === 'recent') return books;
  const sorted = [...books];
  if (sort === 'title') {
    sorted.sort((left, right) => left.title.localeCompare(right.title, undefined, { numeric: true }));
  } else {
    sorted.sort((left, right) => right.progress - left.progress);
  }
  return sorted;
}

export function buildLibraryShelfViewModel(filteredBooks: Book[], books: Book[]): LibraryShelfViewModel {
  return {
    currentBook: filteredBooks[0] ?? books[0] ?? null,
    continueShelf: filteredBooks.slice(1, 6),
    catalogBooks: filteredBooks,
  };
}

export function resolveWorkbenchMainClass(surfaceStyle: 'cards' | 'canvas') {
  return surfaceStyle === 'canvas'
    ? 'library-main-column library-main-column-canvas space-y-6'
    : 'library-main-column library-main-column-plain space-y-6';
}

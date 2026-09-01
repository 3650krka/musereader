import { invoke } from '@tauri-apps/api/core';
import type { BilingualSegment, Book, BookProfile, ReaderTocItem, TranslationTask } from '@/types';

const UI = {
  original: '原文',
  unknownAuthor: '佚名',
} as const;

interface EpubBookMetadata {
  title?: string;
  authors: string[];
}

/** 从 translated.md 的 YAML front matter 提取书名/作者（EPUB 真实元数据，替代哈希文件名）。 */
function parseFrontMatterMeta(content: string): { metaTitle?: string; metaAuthor?: string } {
  const match = content.match(/^---\r?\n([\s\S]*?)\r?\n---/);
  if (!match) return {};
  let metaTitle: string | undefined;
  let metaAuthor: string | undefined;
  for (const line of match[1].split(/\r?\n/)) {
    const kv = line.match(/^([A-Za-z_]+)\s*:\s*(.+?)\s*$/);
    if (!kv) continue;
    const value = kv[2].replace(/^["']|["']$/g, '');
    if (!value) continue;
    if (kv[1] === 'title') metaTitle = value;
    else if (kv[1] === 'author' || kv[1] === 'authors' || kv[1] === 'creator') metaAuthor = value;
  }
  return { metaTitle, metaAuthor };
}

/** 已译完可读书判定：传统翻译任务有 translated.md；外部迁移导入的成书只有 EPUB 本体。 */
function isReadableFinishedTask(task: TranslationTask): boolean {
  if (task.status !== 'completed') return false;
  if (task.outputPath) return true;
  const epub = task.artifactPaths;
  return Boolean(epub?.translatedEpubPath || epub?.bilingualEpubPath || isEpubPath(task.pdfPath));
}

export async function loadCompletedBooks(taskList: TranslationTask[]) {
  const completedTasks = taskList.filter(isReadableFinishedTask);
  const books = await Promise.all(completedTasks.map(loadBookSafely));
  return books.filter((book): book is Book => book !== null);
}

export async function loadMissingBooks(tasks: TranslationTask[], existingBooks: Book[]) {
  const knownIds = new Set(existingBooks.map((book) => book.id));
  const completedTasks = tasks.filter((task) => isReadableFinishedTask(task) && !knownIds.has(task.id));

  const books = await Promise.all(completedTasks.map(loadBookSafely));
  return books.filter((book): book is Book => book !== null);
}

async function loadBookSafely(task: TranslationTask): Promise<Book | null> {
  if (!isReadableFinishedTask(task)) return null;
  try {
    const [content, toc, segments, metadata, cover] = await Promise.all([
      // 迁移导入的成书没有 markdown 工件：正文直接由 EPUB 阅读器从真 XHTML 渲染。
      task.outputPath
        ? invoke<string>('read_translated_file', { path: task.outputPath })
        : Promise.resolve(''),
      loadBookToc(task),
      loadBilingualSegments(task),
      loadEpubMetadata(task),
      resolveCoverUrl(task.coverPath),
    ]);
    return createBookFromTask(task, content, toc, segments, metadata, cover);
  } catch (error) {
    // 单本损坏不应拖垮整个书库。
    console.error(`Failed to load book for task ${task.id}:`, error);
    return null;
  }
}

export function sameTaskError(
  previous?: TranslationTask['lastError'],
  next?: TranslationTask['lastError'],
) {
  if (previous === next) return true;
  if (!previous || !next) return previous === next;

  return (
    previous.code === next.code &&
    previous.message === next.message &&
    previous.retryable === next.retryable &&
    previous.retryAfterMs === next.retryAfterMs
  );
}

export function sameArtifactPaths(
  previous: TranslationTask['artifactPaths'],
  next: TranslationTask['artifactPaths'],
) {
  return (
    previous.artifactDir === next.artifactDir &&
    previous.sourceMarkdownPath === next.sourceMarkdownPath &&
    previous.sourceTocPath === next.sourceTocPath &&
    previous.translatedTocPath === next.translatedTocPath &&
    previous.outputMarkdownPath === next.outputMarkdownPath &&
    previous.outputHtmlPath === next.outputHtmlPath &&
    previous.translatedEpubPath === next.translatedEpubPath &&
    previous.bilingualEpubPath === next.bilingualEpubPath &&
    previous.checkpointPath === next.checkpointPath &&
    previous.manifestPath === next.manifestPath &&
    previous.eventLogPath === next.eventLogPath &&
    previous.validationReportPath === next.validationReportPath &&
    previous.glossaryPath === next.glossaryPath
  );
}

export function sameTask(previous: TranslationTask, next: TranslationTask) {
  return (
    previous.id === next.id &&
    previous.filename === next.filename &&
    previous.pdfPath === next.pdfPath &&
    previous.status === next.status &&
    previous.phase === next.phase &&
    previous.progress === next.progress &&
    previous.message === next.message &&
    previous.articleType === next.articleType &&
    previous.concurrent === next.concurrent &&
    previous.maxWorkers === next.maxWorkers &&
    previous.createdAt === next.createdAt &&
    previous.updatedAt === next.updatedAt &&
    previous.outputPath === next.outputPath &&
    previous.htmlOutputPath === next.htmlOutputPath &&
    previous.coverPath === next.coverPath &&
    previous.totalChunks === next.totalChunks &&
    previous.translatedChunks === next.translatedChunks &&
    previous.retryCount === next.retryCount &&
    previous.sourceHash === next.sourceHash &&
    sameArtifactPaths(previous.artifactPaths, next.artifactPaths) &&
    sameTaskError(previous.lastError, next.lastError)
  );
}

export function mergeTaskUpdates(tasks: TranslationTask[], updates: TranslationTask[]) {
  const updatesById = new Map(updates.map((task) => [task.id, task]));
  let changed = false;
  const merged = tasks.map((task) => {
    const candidate = updatesById.get(task.id);
    if (!candidate) {
      return task;
    }
    if (sameTask(task, candidate)) {
      return task;
    }

    changed = true;
    return candidate;
  });

  return changed ? merged : tasks;
}

export function upsertTaskEntry(tasks: TranslationTask[], incomingTask: TranslationTask) {
  const existingIndex = tasks.findIndex((task) => task.id === incomingTask.id);
  if (existingIndex === -1) {
    return [incomingTask, ...tasks];
  }

  return tasks.map((task) => {
    if (task.id !== incomingTask.id) {
      return task;
    }
    return sameTask(task, incomingTask) ? task : incomingTask;
  });
}

async function loadBookToc(task: TranslationTask): Promise<ReaderTocItem[]> {
  const translatedPath = task.artifactPaths.translatedTocPath;
  const sourcePath = task.artifactPaths.sourceTocPath;
  if (!translatedPath && !sourcePath) {
    return [];
  }

  try {
    return await invoke<ReaderTocItem[]>('read_reader_toc', {
      translatedTocPath: translatedPath ?? null,
      sourceTocPath: sourcePath ?? null,
    });
  } catch (error) {
    console.error('Failed to load reader toc:', error);
    return [];
  }
}

async function loadBilingualSegments(task: TranslationTask): Promise<BilingualSegment[]> {
  // 双语段对从 markdown 工件读取；迁移导入的成书无此工件，由阅读器直接解析包内 span。
  if (!task.outputPath) return [];
  try {
    return await invoke<BilingualSegment[]>('read_bilingual_pairs', { taskId: task.id });
  } catch (error) {
    console.error('Failed to load bilingual segments:', error);
    return [];
  }
}

async function loadEpubMetadata(task: TranslationTask): Promise<EpubBookMetadata | null> {
  const path = task.artifactPaths.bilingualEpubPath
    ?? task.artifactPaths.translatedEpubPath
    ?? (isEpubPath(task.pdfPath) ? task.pdfPath : undefined);
  if (!path) return null;

  try {
    return await invoke<EpubBookMetadata>('read_epub_metadata', { path });
  } catch (error) {
    console.warn(`Failed to read EPUB metadata for task ${task.id}:`, error);
    return null;
  }
}

const IS_TAURI = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** 封面解析：asset 协议 scope 默认拒绝任意磁盘读取（安全面），因此 Tauri 下走
    白名单命令 `read_cover_data_url`（后端限定工件根内文件）返回 data URL；
    web 预览夹具的模块 URL/已是 URL 的原样透传；失败回空串走 hue 占位。 */
async function resolveCoverUrl(coverPath?: string): Promise<string> {
  const raw = coverPath ?? '';
  if (!raw) return '';
  if (/^(https?:|data:|blob:|asset:|\/)/i.test(raw)) return raw;
  if (!IS_TAURI) return raw;
  try {
    return await invoke<string>('read_cover_data_url', { path: raw });
  } catch {
    return '';
  }
}

function isEpubPath(path: string): boolean {
  return /\.epub$/i.test(path.trim());
}

function formatLabel(path: string): string {
  const extension = path.match(/\.([^.\\/]+)$/)?.[1]?.toUpperCase();
  if (extension === 'MARKDOWN') return 'MD';
  return extension ?? '';
}

function createBookFromTask(
  task: TranslationTask,
  content: string,
  toc: ReaderTocItem[],
  segments: BilingualSegment[],
  epubMetadata: EpubBookMetadata | null,
  coverUrl: string,
): Book {
  const frontMatter = parseFrontMatterMeta(content);
  const metadataAuthor = epubMetadata?.authors.map((author) => author.trim()).filter(Boolean).join(', ');
  const metaTitle = epubMetadata?.title?.trim() || frontMatter.metaTitle;
  const metaAuthor = metadataAuthor || frontMatter.metaAuthor;
  const primaryArtifactPath = task.artifactPaths.bilingualEpubPath
    ?? task.artifactPaths.translatedEpubPath;

  return {
    id: task.id,
    title: task.filename.replace(/\.(pdf|epub|docx|md|markdown|txt)$/i, ''),
    author: metaAuthor ?? UI.unknownAuthor,
    cover: coverUrl,
    progress: 0,
    lastRead: '',
    category: task.articleType,
    level: UI.original,
    content,
    originalPath: task.pdfPath,
    primaryArtifactPath,
    translatedEpubPath: task.artifactPaths.translatedEpubPath,
    format: formatLabel(primaryArtifactPath ?? task.filename),
    tocCount: toc.length,
    toc,
    segments,
    metaTitle,
    metaAuthor,
  };
}

/** 书籍档案（收藏/标签）merge 进书籍列表：无档案的书保持原引用。 */
export function applyBookProfiles(books: Book[], profiles: BookProfile[]): Book[] {
  if (!Array.isArray(profiles) || profiles.length === 0) return books;
  const byId = new Map(profiles.map((profile) => [profile.bookId, profile]));
  return books.map((book) => {
    const profile = byId.get(book.id);
    if (!profile) return book;
    const favorite = profile.favorite;
    const tags = profile.tags;
    const folder = profile.folder ?? '';
    if (
      (book.favorite ?? false) === favorite
      && (book.tags ?? []).join(' ') === tags.join(' ')
      && (book.folder ?? '') === folder
    ) {
      return book;
    }
    return { ...book, favorite, tags, folder };
  });
}

export function upsertBookProfile(profiles: BookProfile[], incoming: BookProfile): BookProfile[] {
  const exists = profiles.some((profile) => profile.bookId === incoming.bookId);
  if (!exists) return [...profiles, incoming];
  return profiles.map((profile) => (profile.bookId === incoming.bookId ? incoming : profile));
}

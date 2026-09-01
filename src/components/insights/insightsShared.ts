import { BookOpen, Flame, LibraryBig, Trophy, type LucideIcon } from 'lucide-react';
import type { Book, ReaderActivity, ReaderState, ReviewLogEntry, TranslationTask } from '@/types';

export const UI = {
  uncategorized: '\u672a\u5206\u7c7b',
  notStarted: '\u672a\u5f00\u59cb',
  reading: '\u9605\u8bfb\u4e2d',
  finished: '\u5df2\u8bfb\u5b8c',
  weeklyMinutes: '\u672c\u5468\u65f6\u957f',
  streak: '\u8fde\u7eed\u9605\u8bfb',
  library: '\u9986\u85cf',
  active: '\u5728\u8bfb',
  weekRecap: '\u672c\u5468\u56de\u987e',
  recent: '\u6700\u8fd1\u63a8\u8fdb',
  noReading: '\u6682\u65e0\u9605\u8bfb\u8bb0\u5f55',
  recapTitle: '\u9605\u8bfb\u56de\u987e',
  curveTitle: '\u8fd1\u4e03\u65e5\u9605\u8bfb\u66f2\u7ebf',
  activeDaysSuffix: '\u5929\u6709\u9605\u8bfb\u8bb0\u5f55',
  peakDay: '\u9ad8\u5cf0\u65e5',
  highest: '\u6700\u9ad8',
  minutes: '\u5206\u949f',
  category: '\u5206\u7c7b\u5206\u5e03',
  books: '\u672c',
  noCategoryData: '\u6682\u65e0\u5206\u7c7b\u6570\u636e',
  stages: '\u9605\u8bfb\u9636\u6bb5',
  averageProgress: '\u5e73\u5747\u8fdb\u5ea6',
  notesAndBookmarks: '\u7b14\u8bb0\u4e0e\u4e66\u7b7e',
  bookmarksSuffix: '\u4e2a\u4e66\u7b7e',
  days: '\u5929',
} as const;

export interface BookWithState extends Book {
  progress: number;
  lastRead: string;
}

export interface ReadingPoint {
  date: string;
  minutes: number;
}

export interface VocabBucket {
  label: string;
  count: number;
}

/** 无 CEFR 词表映射时的兜底分桶：按词长近似难度（基础/进阶/高阶）。 */
export function bucketVocabWords(words: string[]): VocabBucket[] {
  const basic = words.filter((w) => w.length <= 5).length;
  const mid = words.filter((w) => w.length >= 6 && w.length <= 9).length;
  const hard = words.filter((w) => w.length >= 10).length;
  return [
    { label: '≤5', count: basic },
    { label: '6–9', count: mid },
    { label: '10+', count: hard },
  ].filter((b) => b.count > 0);
}

export interface OverviewStat {
  label: string;
  value: string;
  unit: string;
  icon: LucideIcon;
}

export interface InsightsSummary {
  booksWithState: BookWithState[];
  activeBooks: BookWithState[];
  finishedBooks: BookWithState[];
  averageProgress: number;
  readingData: ReadingPoint[];
  totalMinutes: number;
  activeDays: number;
  streak: number;
  totalNotes: number;
  totalBookmarks: number;
  topCategories: Array<[string, number]>;
  progressBuckets: Array<{ label: string; count: number }>;
  recentBooks: BookWithState[];
  completedTranslations: number;
  leadBook: BookWithState | null;
  peakDay?: ReadingPoint;
  overviewStats: OverviewStat[];
}

export function buildInsightsSummary(
  books: Book[],
  readerStates: ReaderState[],
  tasks: TranslationTask[],
  rangeDays = 7,
): InsightsSummary {
  const booksWithState = mergeBooksWithState(books, readerStates);
  const activeBooks = booksWithState.filter((book) => book.progress > 0 && book.progress < 100);
  const finishedBooks = booksWithState.filter((book) => book.progress >= 100);
  const averageProgress = calculateAverageProgress(booksWithState);
  const allActivities = collectActivities(readerStates);
  const completedTranslations = tasks.filter((task) => task.status === 'completed').length;
  const readingData = buildReadingData(allActivities, rangeDays);
  const totalMinutes = readingData.reduce((sum, item) => sum + item.minutes, 0);
  const activeDays = readingData.filter((item) => item.minutes > 0).length;
  const streak = calculateStreak(readingData);
  const totalNotes = readerStates.reduce((sum, state) => sum + state.notes.length, 0);
  const totalBookmarks = readerStates.reduce((sum, state) => sum + state.bookmarks.length, 0);
  const topCategories = buildTopCategories(booksWithState);
  const progressBuckets = [
    { label: UI.notStarted, count: booksWithState.filter((book) => book.progress === 0).length },
    { label: UI.reading, count: activeBooks.length },
    { label: UI.finished, count: finishedBooks.length },
  ];
  const recentBooks = buildRecentBooks(booksWithState, readerStates);
  const leadBook = recentBooks[0] ?? activeBooks[0] ?? booksWithState[0] ?? null;
  const peakDay = readingData.reduce<ReadingPoint | undefined>(
    (peak, point) => point.minutes > 0 && (!peak || point.minutes > peak.minutes) ? point : peak,
    undefined,
  );
  const overviewStats = [
    { label: UI.weeklyMinutes, value: String(totalMinutes), unit: UI.minutes, icon: Flame },
    { label: UI.streak, value: String(streak), unit: UI.days, icon: Trophy },
    { label: UI.library, value: String(booksWithState.length), unit: UI.books, icon: LibraryBig },
    { label: UI.active, value: String(activeBooks.length), unit: UI.books, icon: BookOpen },
  ];

  return {
    booksWithState,
    activeBooks,
    finishedBooks,
    averageProgress,
    readingData,
    totalMinutes,
    activeDays,
    streak,
    totalNotes,
    totalBookmarks,
    topCategories,
    progressBuckets,
    recentBooks,
    completedTranslations,
    leadBook,
    peakDay,
    overviewStats,
  };
}

function mergeBooksWithState(books: Book[], readerStates: ReaderState[]): BookWithState[] {
  const stateMap = new Map(readerStates.map((state) => [state.bookId, state]));
  return books.map((book) => {
    const state = stateMap.get(book.id);
    return state
      ? {
          ...book,
          progress: state.progress,
          lastRead: formatLastActive(state.updatedAt),
        }
      : book;
  });
}

function calculateAverageProgress(books: BookWithState[]) {
  const totalProgress = books.reduce((sum, book) => sum + book.progress, 0);
  return books.length > 0 ? Math.round(totalProgress / books.length) : 0;
}

function buildReadingData(
  allActivities: ReaderActivity[],
  days = 7,
): ReadingPoint[] {
  return Array.from({ length: days }, (_, index) => {
    const date = new Date();
    date.setDate(date.getDate() - (days - 1 - index));

    const activityMinutes = sumMinutesForDay(allActivities, date);
    if (activityMinutes > 0) {
      return { date: formatDateLabel(date), minutes: activityMinutes };
    }

    return { date: formatDateLabel(date), minutes: 0 };
  });
}

export interface BookMinutes {
  bookId: string;
  minutes: number;
}

/** 每本书累计阅读分钟（activities 按 bookId 聚合，降序）。 */
export function buildBookMinutes(readerStates: ReaderState[]): BookMinutes[] {
  return readerStates
    .map((state) => ({
      bookId: state.bookId,
      minutes: state.activities.reduce((sum, activity) => sum + activity.minutes, 0),
    }))
    .filter((item) => item.minutes > 0)
    .sort((left, right) => right.minutes - left.minutes);
}

/** 复习趋势（reviewLog 按日聚合）：minutes 字段复用为当日复习次数。 */
export function buildReviewData(
  reviewLog: ReviewLogEntry[],
  days = 7,
): ReadingPoint[] {
  const counts = new Map<string, number>();
  for (const entry of reviewLog) {
    const ts = new Date(entry.createdAt);
    if (Number.isNaN(ts.getTime())) continue;
    const key = `${ts.getFullYear()}-${ts.getMonth()}-${ts.getDate()}`;
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  return Array.from({ length: days }, (_, index) => {
    const date = new Date();
    date.setDate(date.getDate() - (days - 1 - index));
    const key = `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
    return { date: formatDateLabel(date), minutes: counts.get(key) ?? 0 };
  });
}

function calculateStreak(readingData: ReadingPoint[]) {
  let count = 0;
  for (let index = readingData.length - 1; index >= 0; index -= 1) {
    if (readingData[index].minutes > 0) {
      count += 1;
    } else {
      break;
    }
  }
  return count;
}

function buildTopCategories(books: BookWithState[]) {
  const categoryCounts = books.reduce<Record<string, number>>((accumulator, book) => {
    const key = book.category || UI.uncategorized;
    accumulator[key] = (accumulator[key] || 0) + 1;
    return accumulator;
  }, {});

  return Object.entries(categoryCounts)
    .sort((left, right) => right[1] - left[1])
    .slice(0, 4);
}

function buildRecentBooks(books: BookWithState[], readerStates: ReaderState[]) {
  const stateMap = new Map(readerStates.map((state) => [state.bookId, state]));
  return [...books]
    .filter((book) => stateMap.has(book.id))
    .sort((left, right) => {
      const leftTime = stateMap.get(left.id) ? new Date(stateMap.get(left.id)!.updatedAt).getTime() : 0;
      const rightTime = stateMap.get(right.id) ? new Date(stateMap.get(right.id)!.updatedAt).getTime() : 0;
      return rightTime - leftTime;
    })
    .slice(0, 5);
}

function formatDateLabel(value: Date) {
  return `${value.getMonth() + 1}/${value.getDate()}`;
}

function sameDay(left: Date, right: Date) {
  return left.toDateString() === right.toDateString();
}

function formatLastActive(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }

  return date.toLocaleDateString('zh-CN', {
    month: 'numeric',
    day: 'numeric',
  });
}

function collectActivities(readerStates: ReaderState[]) {
  return readerStates.flatMap((state) => state.activities ?? []);
}

function sumMinutesForDay(activities: ReaderActivity[], targetDate: Date) {
  return activities.reduce((sum, activity) => {
    const createdAt = new Date(activity.createdAt);
    if (Number.isNaN(createdAt.getTime())) return sum;
    return sameDay(createdAt, targetDate) ? sum + activity.minutes : sum;
  }, 0);
}

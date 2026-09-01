import type { Book, ReaderTocItem } from '@/types';
import type { ReaderLocation } from './epubReaderTypes';

export function orderedReaderToc(toc: ReaderTocItem[]): ReaderTocItem[] {
  return [...toc].sort((left, right) => left.progress - right.progress);
}

export function chapterAtProgress(
  orderedToc: ReaderTocItem[],
  ratio: number,
  fallback: string,
): string {
  const progress = ratio * 100;
  let current = fallback;
  for (const item of orderedToc) {
    if (item.progress > progress) break;
    current = item.title || current;
  }
  return current;
}

export function findReaderAnchor(doc: Document | null, anchor: string): Element | null {
  if (!doc || !anchor) return null;
  const byId = doc.getElementById(anchor);
  if (byId) return byId;
  for (const element of doc.querySelectorAll('[data-mt-idx]')) {
    if (element.getAttribute('data-mt-idx') === anchor) return element;
  }
  return null;
}

export function readerTitle(book: Book): string {
  return book.metaTitle?.trim() || book.title;
}

export function sameLocation(
  left: ReaderLocation | null,
  right: ReaderLocation,
): boolean {
  return left?.href === right.href && left.offset === right.offset;
}

export function safeDecode(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

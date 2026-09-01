import type { Book } from '@/types';

function normalizeText(value: string | undefined): string {
  return value?.replace(/\s+/g, ' ').trim() ?? '';
}

function normalizeMarkdownBlock(block: string): string {
  return block
    .replace(/!\[[^\]]*\]\([^)]*\)/g, ' ')
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/<[^>]+>/g, ' ')
    .replace(/^\s{0,3}(?:#{1,6}|>|[-*+]|\d+[.)])\s+/gm, '')
    .replace(/[*_~`]/g, '')
    .replace(/\s+/g, ' ')
    .trim();
}

function truncateText(value: string, maxLength: number): string {
  if (value.length <= maxLength) return value;
  return `${value.slice(0, maxLength).trimEnd()}…`;
}

function proseBlocks(markdown: string, title: string): string[] {
  const blocks: string[] = [];
  for (const block of markdown.split(/\r?\n\s*\r?\n/)) {
    if (/^\s{0,3}#{1,6}\s+/.test(block)) continue;
    const text = normalizeMarkdownBlock(block);
    if (text.length < 40 || normalizeText(text) === title) continue;
    blocks.push(text);
  }
  return blocks;
}

function selectContentSection(body: string, title: string): string {
  const sections = body.split(/(?=^\s{0,3}#{1,2}\s+)/m);
  for (const section of sections) {
    const blocks = proseBlocks(section, title);
    const proseLength = blocks.reduce((total, block) => total + block.length, 0);
    if (proseLength >= 320) return section;
  }
  return body;
}

export function displayBookTitle(book: Book, fallback: string): string {
  const metadataTitle = normalizeText(book.metaTitle);
  if (metadataTitle) return metadataTitle;

  const stem = book.title.replace(/\.(pdf|epub|docx|md|markdown|txt)$/i, '');
  const readableStem = normalizeText(stem.replace(/[_-]+/g, ' '));
  return readableStem || fallback;
}

export function displayBookExcerpt(book: Book, maxLength = 180): string {
  const body = (book.content ?? '')
    .replace(/^---\s*\r?\n[\s\S]*?\r?\n---\s*(?:\r?\n|$)/, '')
    .replace(/<\/(?:p|div|section|article|h[1-6])>/gi, '\n\n');
  const title = normalizeText(displayBookTitle(book, ''));
  const section = selectContentSection(body, title);
  const firstBlock = proseBlocks(section, title)[0];
  return firstBlock ? truncateText(firstBlock, maxLength) : '';
}

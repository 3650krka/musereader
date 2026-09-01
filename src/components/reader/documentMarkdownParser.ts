export type MarkdownBlock =
  | { kind: 'heading'; level: number; text: string }
  | { kind: 'paragraph'; text: string }
  | { kind: 'quote'; lines: string[] }
  | { kind: 'list'; ordered: boolean; items: string[] }
  | { kind: 'code'; language: string; text: string }
  | { kind: 'image'; alt: string; src: string }
  | { kind: 'table'; rows: string[][] }
  | { kind: 'divider' };

interface ParsedBlock {
  block: MarkdownBlock;
  nextIndex: number;
}

export function parseDocumentMarkdown(markdown: string): MarkdownBlock[] {
  const lines = stripFrontMatter(markdown)
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/\r\n?/g, '\n')
    .split('\n');
  const blocks: MarkdownBlock[] = [];
  let index = 0;

  while (index < lines.length) {
    if (!lines[index].trim() || lines[index].trim() === ':::') {
      index += 1;
      continue;
    }
    const parsed = parseBlockAt(lines, index);
    blocks.push(parsed.block);
    index = parsed.nextIndex;
  }
  return blocks;
}

export function parseMarkdownImage(value: string): { alt: string; src: string } | null {
  const match = value.match(/^!\[([^\]]*)]\(([^)]+)\)$/);
  return match ? { alt: match[1], src: match[2].trim() } : null;
}

function parseBlockAt(lines: string[], index: number): ParsedBlock {
  const line = lines[index];
  const fence = line.match(/^\s*```\s*([^\s`]*)\s*$/);
  if (fence) return collectCodeBlock(lines, index, fence[1] ?? '');

  const heading = line.match(/^\s*(#{1,6})\s+(.+?)\s*#*\s*$/);
  if (heading) {
    return {
      block: { kind: 'heading', level: heading[1].length, text: heading[2] },
      nextIndex: index + 1,
    };
  }
  if (/^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/.test(line)) {
    return { block: { kind: 'divider' }, nextIndex: index + 1 };
  }
  if (/^\s*>/.test(line)) return collectQuote(lines, index);
  if (listMatch(line)) return collectList(lines, index);
  if (isTableStart(lines, index)) return collectTable(lines, index);

  const image = parseMarkdownImage(line.trim());
  if (image) return { block: { kind: 'image', ...image }, nextIndex: index + 1 };
  return collectParagraph(lines, index);
}

function collectCodeBlock(lines: string[], index: number, language: string): ParsedBlock {
  const content: string[] = [];
  let cursor = index + 1;
  while (cursor < lines.length && !/^\s*```\s*$/.test(lines[cursor])) {
    content.push(lines[cursor]);
    cursor += 1;
  }
  return {
    block: { kind: 'code', language, text: content.join('\n') },
    nextIndex: Math.min(lines.length, cursor + 1),
  };
}

function collectQuote(lines: string[], index: number): ParsedBlock {
  const content: string[] = [];
  let cursor = index;
  while (cursor < lines.length) {
    const match = lines[cursor].match(/^\s*>\s?(.*)$/);
    if (!match) break;
    content.push(match[1]);
    cursor += 1;
  }
  return { block: { kind: 'quote', lines: content }, nextIndex: cursor };
}

function collectList(lines: string[], index: number): ParsedBlock {
  const first = listMatch(lines[index]);
  const ordered = Boolean(first?.ordered);
  const items: string[] = [];
  let cursor = index;
  while (cursor < lines.length) {
    const match = listMatch(lines[cursor]);
    if (!match || Boolean(match.ordered) !== ordered) break;
    items.push(match.text);
    cursor += 1;
  }
  return { block: { kind: 'list', ordered, items }, nextIndex: cursor };
}

function collectTable(lines: string[], index: number): ParsedBlock {
  const rows = [splitTableRow(lines[index])];
  let cursor = index + 2;
  while (cursor < lines.length && lines[cursor].includes('|') && lines[cursor].trim()) {
    rows.push(splitTableRow(lines[cursor]));
    cursor += 1;
  }
  return { block: { kind: 'table', rows }, nextIndex: cursor };
}

function collectParagraph(lines: string[], index: number): ParsedBlock {
  const content: string[] = [];
  let cursor = index;
  while (cursor < lines.length && lines[cursor].trim()) {
    if (cursor > index && isBlockBoundary(lines, cursor)) break;
    content.push(lines[cursor].trim());
    cursor += 1;
  }
  return {
    block: { kind: 'paragraph', text: content.join(' ') },
    nextIndex: cursor,
  };
}

function listMatch(line: string): { ordered: boolean; text: string } | null {
  const ordered = line.match(/^\s*\d+[.)]\s+(.+)$/);
  if (ordered) return { ordered: true, text: ordered[1] };
  const unordered = line.match(/^\s*[-+*]\s+(.+)$/);
  return unordered ? { ordered: false, text: unordered[1] } : null;
}

function isBlockBoundary(lines: string[], index: number): boolean {
  const line = lines[index];
  return Boolean(
    /^\s*```/.test(line) ||
      /^\s*#{1,6}\s+/.test(line) ||
      /^\s*>/.test(line) ||
      listMatch(line) ||
      isTableStart(lines, index) ||
      parseMarkdownImage(line.trim()) ||
      /^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/.test(line),
  );
}

function isTableStart(lines: string[], index: number): boolean {
  if (index + 1 >= lines.length || !lines[index].includes('|')) return false;
  return /^\s*\|?\s*:?-{3,}:?\s*(?:\|\s*:?-{3,}:?\s*)+\|?\s*$/.test(lines[index + 1]);
}

function splitTableRow(line: string): string[] {
  const normalized = line.trim().replace(/^\|/, '').replace(/\|$/, '');
  return normalized.split(/(?<!\\)\|/).map((cell) => cell.trim().replace(/\\\|/g, '|'));
}

function stripFrontMatter(markdown: string): string {
  if (!/^---\r?\n/.test(markdown)) return markdown;
  const end = markdown.slice(4).search(/\r?\n---(?:\r?\n|$)/);
  return end < 0 ? markdown : markdown.slice(end + 8);
}

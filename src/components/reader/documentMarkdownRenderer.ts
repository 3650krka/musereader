import {
  parseDocumentMarkdown,
  parseMarkdownImage,
  type MarkdownBlock,
} from './documentMarkdownParser';

const INLINE_TOKEN_PATTERN =
  /(`[^`\n]+`|!\[[^\]\n]*]\([^)]+\)|\[[^\]\n]+]\([^)]+\)|\*\*[^*\n]+\*\*|__[^_\n]+__|\*[^*\n]+\*|_[^_\n]+_)/g;

export function renderMarkdown(markdown: string, includeAnchors = true): string {
  const blocks = parseDocumentMarkdown(markdown);
  const headingCounts = new Map<string, number>();
  return blocks
    .map((block, index) => renderBlock(block, index, includeAnchors, headingCounts))
    .join('\n');
}

export function escapeHtml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

export function escapeHtmlAttribute(value: string): string {
  return escapeHtml(value).replace(/`/g, '&#96;');
}

function renderBlock(
  block: MarkdownBlock,
  index: number,
  includeAnchors: boolean,
  headingCounts: Map<string, number>,
): string {
  if (block.kind === 'heading') {
    const id = includeAnchors ? headingId(block.text, index, headingCounts) : '';
    return `<h${block.level}${id ? ` id="${escapeHtmlAttribute(id)}"` : ''}>${renderInline(block.text)}</h${block.level}>`;
  }
  if (block.kind === 'paragraph') return `<p>${renderInline(block.text)}</p>`;
  if (block.kind === 'quote') {
    return `<blockquote>${block.lines.map(renderInline).join('<br />')}</blockquote>`;
  }
  if (block.kind === 'list') {
    const tag = block.ordered ? 'ol' : 'ul';
    return `<${tag}>${block.items.map((item) => `<li>${renderInline(item)}</li>`).join('')}</${tag}>`;
  }
  if (block.kind === 'code') {
    const language = block.language.replace(/[^A-Za-z0-9_-]/g, '');
    return `<pre><code${language ? ` class="language-${language}"` : ''}>${escapeHtml(block.text)}</code></pre>`;
  }
  if (block.kind === 'image') return renderImage(block.alt, block.src);
  if (block.kind === 'table') return renderTable(block.rows);
  return '<hr />';
}

function renderTable(rows: string[][]): string {
  const [header = [], ...body] = rows;
  return `<div class="mt-table-wrap"><table><thead><tr>${header
    .map((cell) => `<th>${renderInline(cell)}</th>`)
    .join('')}</tr></thead><tbody>${body
    .map((row) => `<tr>${row.map((cell) => `<td>${renderInline(cell)}</td>`).join('')}</tr>`)
    .join('')}</tbody></table></div>`;
}

function renderInline(value: string): string {
  let output = '';
  let cursor = 0;
  for (const match of value.matchAll(INLINE_TOKEN_PATTERN)) {
    const start = match.index ?? 0;
    output += escapeHtml(value.slice(cursor, start));
    output += renderInlineToken(match[0]);
    cursor = start + match[0].length;
  }
  return output + escapeHtml(value.slice(cursor));
}

function renderInlineToken(token: string): string {
  const image = parseMarkdownImage(token);
  if (image) return renderImage(image.alt, image.src);
  const link = token.match(/^\[([^\]]+)]\(([^)]+)\)$/);
  if (link) {
    const href = safeUrl(link[2]);
    return href
      ? `<a href="${escapeHtmlAttribute(href)}">${escapeHtml(link[1])}</a>`
      : escapeHtml(link[1]);
  }
  if (token.startsWith('`')) return `<code>${escapeHtml(token.slice(1, -1))}</code>`;
  if (token.startsWith('**') || token.startsWith('__')) {
    return `<strong>${escapeHtml(token.slice(2, -2))}</strong>`;
  }
  return `<em>${escapeHtml(token.slice(1, -1))}</em>`;
}

function renderImage(alt: string, source: string): string {
  const src = safeUrl(source);
  if (!src) return alt ? `<span>${escapeHtml(alt)}</span>` : '';
  return `<figure><img src="${escapeHtmlAttribute(src)}" alt="${escapeHtmlAttribute(alt)}" loading="lazy" />${alt ? `<figcaption>${escapeHtml(alt)}</figcaption>` : ''}</figure>`;
}

function headingId(text: string, index: number, counts: Map<string, number>): string {
  const base = text
    .replace(/[`*_\[\]()#!]/g, ' ')
    .normalize('NFKC')
    .toLocaleLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, '-')
    .replace(/^-+|-+$/g, '') || `section-${index + 1}`;
  const count = counts.get(base) ?? 0;
  counts.set(base, count + 1);
  return count === 0 ? base : `${base}-${count + 1}`;
}

function safeUrl(value: string): string {
  const normalized = value.trim().replace(/^<|>$/g, '');
  if (/^(?:javascript|vbscript):/i.test(normalized)) return '';
  if (/^data:/i.test(normalized) && !/^data:image\//i.test(normalized)) return '';
  return normalized;
}

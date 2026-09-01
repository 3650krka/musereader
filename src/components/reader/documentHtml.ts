import type { BilingualSegment } from '@/types';
import type { ReaderLang } from './epubReaderTypes';
import {
  escapeHtml,
  escapeHtmlAttribute,
  renderMarkdown,
} from './documentMarkdownRenderer';

interface DocumentHtmlOptions {
  title: string;
  content: string;
  segments: BilingualSegment[];
  lang: ReaderLang;
}

export function buildDocumentReaderHtml(options: DocumentHtmlOptions): string {
  const body = buildDocumentBody(options);
  return `<!doctype html>
<html lang="${options.lang === 'en' ? 'en' : 'zh-CN'}">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>${escapeHtml(options.title)}</title>
  <style>${DOCUMENT_BASE_CSS}</style>
</head>
<body>${body}</body>
</html>`;
}

function buildDocumentBody(options: DocumentHtmlOptions): string {
  if (options.lang === 'both' && options.segments.length > 0) {
    return renderBilingualSegments(options.segments);
  }
  if (options.lang === 'en' && options.segments.length > 0) {
    return renderMarkdown(options.segments.map((segment) => segment.source).join('\n\n'));
  }
  const content = options.content.trim() || `# ${options.title}`;
  return renderMarkdown(content);
}

function renderBilingualSegments(segments: BilingualSegment[]): string {
  return segments
    .filter((segment) => segment.source.trim() || segment.target.trim())
    .map((segment, index) => {
      const key = segment.anchorId?.trim() || `segment-${index + 1}`;
      return `<section data-mt-pair data-mt-idx="${escapeHtmlAttribute(key)}" id="mt-segment-${index + 1}">
  <div class="mt-zh">${renderMarkdown(segment.target, false)}</div>
  <div class="mt-en">${renderMarkdown(segment.source, false)}</div>
</section>`;
    })
    .join('\n');
}

const DOCUMENT_BASE_CSS = `
html,body{margin:0;padding:0;}
body{font-family:ui-serif,"Source Han Serif SC","Noto Serif CJK SC",serif;text-rendering:optimizeLegibility;}
h1,h2,h3,h4,h5,h6{break-after:avoid;break-inside:avoid;line-height:1.3;}
h1{font-size:1.75em;margin:0 0 1.1em;}
h2{font-size:1.38em;margin:1.5em 0 .75em;}
h3{font-size:1.16em;margin:1.25em 0 .65em;}
p,li,blockquote{orphans:2;widows:2;}
ul,ol{padding-inline-start:1.5em;}
blockquote{margin-inline:0;padding-inline-start:1em;border-inline-start:1px solid currentColor;opacity:.88;}
figure{margin:1.1em 0;break-inside:avoid;text-align:center;}
figcaption{margin-top:.5em;font-size:.82em;opacity:.72;}
pre{padding:1em;border-radius:.4em;background:rgba(90,86,78,.08);white-space:pre-wrap;overflow-wrap:anywhere;}
code{font-family:ui-monospace,"Cascadia Mono",monospace;font-size:.88em;}
.mt-table-wrap{max-width:100%;overflow:hidden;break-inside:avoid;}
table{width:100%;border-collapse:collapse;font-size:.86em;}
th,td{padding:.45em .55em;border:1px solid rgba(100,96,88,.25);text-align:start;vertical-align:top;}
section[data-mt-pair]{margin:0 0 max(var(--rd-para-spacing,1.2em), var(--rd-gap-floor,0px));break-inside:auto;}
section[data-mt-pair]>.mt-zh,section[data-mt-pair]>.mt-en{min-width:0;}
section[data-mt-pair] h1,section[data-mt-pair] h2,section[data-mt-pair] h3{margin-top:0;}
`.trim();

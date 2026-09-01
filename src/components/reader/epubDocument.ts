import { convertFileSrc } from '@tauri-apps/api/core';

export interface ReaderSectionDocument {
  assetUrl: string;
  key: string;
  srcDoc: string;
}

const BASE_TAG_PATTERN = /<base\b[^>]*>/gi;
const HEAD_OPEN_PATTERN = /<head\b[^>]*>/i;
const HTML_OPEN_PATTERN = /<html\b[^>]*>/i;

export async function loadReaderSectionDocument(
  filePath: string,
  sectionKey: string,
  signal: AbortSignal,
): Promise<ReaderSectionDocument> {
  const assetUrl = convertFileSrc(filePath);
  const response = await fetch(assetUrl, { signal });
  if (!response.ok && response.status !== 0) {
    throw new Error(`读取阅读内容失败（${response.status}）`);
  }

  const source = await response.text();
  if (!source.trim()) {
    throw new Error('阅读内容为空');
  }

  return {
    assetUrl,
    key: `${sectionKey}:${assetUrl}`,
    srcDoc: injectReaderBase(source, assetUrl),
  };
}

export function injectReaderBase(source: string, assetUrl: string): string {
  const sanitizedSource = source.replace(BASE_TAG_PATTERN, '');
  const baseTag = `<base data-mt-reader-base href="${escapeHtmlAttribute(
    sectionDirectoryUrl(assetUrl),
  )}" />`;
  const headOpen = HEAD_OPEN_PATTERN.exec(sanitizedSource);

  if (headOpen?.index !== undefined) {
    const insertAt = headOpen.index + headOpen[0].length;
    return `${sanitizedSource.slice(0, insertAt)}${baseTag}${sanitizedSource.slice(
      insertAt,
    )}`;
  }

  const htmlOpen = HTML_OPEN_PATTERN.exec(sanitizedSource);
  if (htmlOpen?.index !== undefined) {
    const insertAt = htmlOpen.index + htmlOpen[0].length;
    return `${sanitizedSource.slice(0, insertAt)}<head>${baseTag}</head>${sanitizedSource.slice(
      insertAt,
    )}`;
  }

  return `<html><head>${baseTag}</head><body>${sanitizedSource}</body></html>`;
}

function sectionDirectoryUrl(assetUrl: string): string {
  const url = new URL(assetUrl);
  url.hash = '';
  url.search = '';
  const hierarchicalPath = decodeURIComponent(url.pathname).replace(/\\/g, '/');
  const slash = hierarchicalPath.lastIndexOf('/');
  url.pathname = slash >= 0 ? hierarchicalPath.slice(0, slash + 1) : '/';
  return url.toString();
}

function escapeHtmlAttribute(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}

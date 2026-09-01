/** 词库富字段解码工具：WordWise 行间词卡与背诵卡背面共用（避免两处各写一份）。 */

/** 考试标签 code → 中文档级名（与后端 WordLevel.tag 取值一致）。 */
const TAG_LABELS: Record<string, string> = {
  zk: '中考',
  gk: '高考',
  cet4: '四级',
  cet6: '六级',
  ky: '考研',
  ielts: '雅思',
  toefl: '托福',
  tem4: '专四',
  tem8: '专八',
  gre: 'GRE',
  sat: 'GRE',
};

/** 空格分隔的标签串 → 展示档名数组（未知标签原样保留）。 */
export function tagLabelsFrom(tag: string | undefined): string[] {
  if (!tag) return [];
  return tag
    .split(/\s+/)
    .filter(Boolean)
    .map((code) => TAG_LABELS[code] ?? code);
}

/** 词形变换 code 集合（d 过去式/p 过去分词/i 进行时/3 三单/s 复数/r 比较级/t 最高级）。 */
const FORM_CODES = ['d', 'p', 'i', '3', 's', 'r', 't'];

/** exchange 编码（"d:abandoned/p:abandoned"）→ {label,value} 列表；
    labels 为 i18n 的 code→文案映射（缺省回退原 code）。 */
export function decodeExchange(
  exchange: string | undefined,
  word: string,
  labels: Record<string, string>,
): Array<{ label: string; value: string }> {
  if (!exchange) return [];
  const out: Array<{ label: string; value: string }> = [];
  for (const part of exchange.split('/')) {
    const sep = part.indexOf(':');
    if (sep < 1) continue;
    const code = part.slice(0, sep);
    const value = part.slice(sep + 1).trim();
    if (!value || value === word) continue;
    if (!FORM_CODES.includes(code)) continue;
    out.push({ label: labels[code] ?? code, value });
  }
  return out;
}

/** BNC/COCA 词频排名 → 简短文案（两侧都有则并列）。 */
export function freqText(info: { bnc?: number; frq?: number }): string {
  const parts: string[] = [];
  if (info.bnc) parts.push(`BNC #${info.bnc.toLocaleString()}`);
  if (info.frq) parts.push(`COCA #${info.frq.toLocaleString()}`);
  return parts.join(' · ');
}

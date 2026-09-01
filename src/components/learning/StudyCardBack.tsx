import { ChevronDown, ChevronUp } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useT } from '@/i18n';
import { decodeExchange, freqText, tagLabelsFrom } from '@/lib/wordCardBits';
import type { VocabCardFace } from '@/types';

/**
 * 背诵卡「背面」：词库信息分层展示。
 *  - 第一层：释义（AI 语境义优先）+ 考试标签 chips + 柯林斯星级；
 *  - 第二层：语境例句（原文 + 可选译文）；
 *  - 折叠层：完整词性分段释义、词形变换、词根助记、词频排名——点开才占空间。
 * 数据缺失（未导入词库/词表未收录）时相应区块自动隐藏。
 */
export function StudyCardBack({ face }: { face: VocabCardFace }) {
  const t = useT();
  const [more, setMore] = useState(false);

  const tags = useMemo(() => tagLabelsFrom(face.tag), [face.tag]);
  const forms = useMemo(
    () => decodeExchange(face.exchange, face.word, t.reader.formLabels),
    [face.exchange, face.word, t.reader.formLabels],
  );
  const freq = freqText({ bnc: face.bnc, frq: face.frq });
  const translation = face.translation && face.translation !== face.definition ? face.translation : '';
  const hasMore = Boolean(translation || forms.length || face.root || freq);

  return (
    <div style={{ marginTop: 18, width: '100%' }}>
      <p style={{ whiteSpace: 'pre-line', fontSize: 14.5, lineHeight: 1.9, margin: 0 }}>
        {face.definition || '—'}
      </p>
      {(tags.length > 0 || Boolean(face.oxford) || Boolean(face.collins)) && (
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6, justifyContent: 'center', marginTop: 10 }}>
          {face.oxford ? <span className="chip">{t.reader.wordCard.oxford}</span> : null}
          {tags.map((label) => <span key={label} className="chip">{label}</span>)}
          {face.collins ? (
            <span className="chip" title={`${t.reader.wordCard.collinsTitle} ${face.collins} ★`}>
              {'★'.repeat(face.collins)}{'☆'.repeat(Math.max(0, 5 - face.collins))}
            </span>
          ) : null}
        </div>
      )}
      {face.context && (
        <p style={{ fontSize: 13, fontStyle: 'italic', lineHeight: 1.8, color: 'var(--ink-3)', marginTop: 12, marginBottom: 0 }}>
          “{face.context}”
        </p>
      )}
      {face.contextZh && (
        <p style={{ fontSize: 13, lineHeight: 1.8, color: 'var(--ink-3)', marginTop: 4, marginBottom: 0 }}>
          {face.contextZh}
        </p>
      )}
      {hasMore && (
        <>
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              setMore((v) => !v);
            }}
            style={{
              display: 'inline-flex', alignItems: 'center', gap: 4, marginTop: 12,
              border: 0, background: 'none', cursor: 'pointer',
              font: 'inherit', fontSize: 12.5, color: 'var(--signal)',
            }}
            aria-expanded={more}
          >
            {more ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
            {more ? t.learning.detailLess : t.learning.detailMore}
          </button>
          {more && (
            <div style={{ textAlign: 'left', marginTop: 8, display: 'grid', gap: 8, fontSize: 13, lineHeight: 1.8 }}>
              {translation && (
                <p style={{ margin: 0, whiteSpace: 'pre-line', color: 'var(--ink-2)' }}>{translation}</p>
              )}
              {forms.length > 0 && (
                <p style={{ margin: 0, color: 'var(--ink-2)' }}>
                  {forms.map((f) => `${f.label} ${f.value}`).join(' · ')}
                </p>
              )}
              {face.root && <p style={{ margin: 0, color: 'var(--ink-2)' }}>{t.reader.wordCard.root}：{face.root}</p>}
              {freq && <p style={{ margin: 0, color: 'var(--ink-3)', fontFamily: 'var(--font-mono)', fontSize: 12 }}>{freq}</p>}
            </div>
          )}
        </>
      )}
    </div>
  );
}

import { invoke } from '@tauri-apps/api/core';
import { Blocks, FilePenLine, Plus, Trash2 } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useT } from '@/i18n';
import type { AgentPromptPreview } from '@/types';
import { useAppDialog } from '@/components/common/AppDialog';
import type { ServicePanelProps } from './panelTypes';

type SkillPromptPanelProps = Pick<
  ServicePanelProps,
  | 'skillEntries'
  | 'agentDefinitions'
  | 'selectedSkillId'
  | 'setSelectedSkillId'
  | 'skillDraft'
  | 'setSkillDraft'
  | 'skillSaveState'
  | 'onSaveSkill'
> & {
  translationPreview: AgentPromptPreview | null;
  readingPreview: AgentPromptPreview | null;
  /** 列表刷新（新增/删除后）。 */
  onEntriesChanged: () => void;
};

/** Prompt 库：翻译用 / 阅读用 两个 agent 分区编辑 + 实时预览。 */
export function SkillPromptPanel({
  skillEntries,
  agentDefinitions,
  translationPreview,
  readingPreview,
  selectedSkillId,
  setSelectedSkillId,
  skillDraft,
  setSkillDraft,
  skillSaveState,
  onSaveSkill,
  onEntriesChanged,
}: SkillPromptPanelProps) {
  const t = useT();
  const { appConfirm, appPrompt } = useAppDialog();
  const [agent, setAgent] = useState<'translation' | 'reading'>('translation');

  const filtered = useMemo(
    () => skillEntries.filter((entry) => entry.agent === agent),
    [skillEntries, agent],
  );

  const deleteEntry = async (skillId: string) => {
    if (!(await appConfirm({ title: t.set.deleteSkillConfirm, danger: true }))) return;
    try {
      await invoke('delete_skill_entry', { skillId });
      onEntriesChanged();
    } catch (error) {
      console.error('delete skill failed:', error);
    }
  };

  const addPrompt = async () => {
    const name = await appPrompt({ title: t.set.addPromptName });
    if (!name?.trim()) return;
    invoke('create_skill_entry', { name: name.trim(), content: '' })
      .then(() => onEntriesChanged())
      .catch((error) => console.error('create prompt failed:', error));
  };
  const selectedSkill = filtered.find((e) => e.id === selectedSkillId) ?? filtered[0] ?? null;
  const preview = agent === 'translation' ? translationPreview : readingPreview;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <div className="card">
        <div className="card-title"><Blocks size={18} /><span>{t.set.translationAgent}</span></div>
        {agentDefinitions.map((a) => (
          <div key={a.id} className="row2" style={{ marginTop: 8 }}>
            <div className="grow">
              <div className="name">{a.title}</div>
              <div className="desc">{a.description}</div>
            </div>
            <span className="chip" style={{ fontSize: 11.5 }}>{a.capabilities.length} {t.set.agentCaps}</span>
          </div>
        ))}
      </div>

      <div className="skill2">
        <div className="card" style={{ padding: 12 }}>
          {/* agent 分区：翻译用 / 阅读用 */}
          <div className="skill-agent-tabs">
            <button
              type="button"
              className={`chip ${agent === 'translation' ? 'on' : ''}`}
              onClick={() => setAgent('translation')}
            >
              {t.set.promptForTranslation}
            </button>
            <button
              type="button"
              className={`chip ${agent === 'reading' ? 'on' : ''}`}
              onClick={() => setAgent('reading')}
            >
              {t.set.promptForReading}
            </button>
          </div>
          <div className="skill-list-scroll">
            {filtered.map((entry) => (
              <div key={entry.id} className="skill-list-row">
                <button
                  type="button"
                  className={`snav ${selectedSkill?.id === entry.id ? 'on' : ''}`}
                  style={{ width: '100%' }}
                  onClick={() => setSelectedSkillId(entry.id)}
                >
                  <FilePenLine size={15} />
                  <span className="truncate">{entry.title}</span>
                </button>
                {entry.kind === 'reference' && (
                  <button
                    type="button"
                    className="skill-del"
                    title={t.set.deleteSkill}
                    onClick={() => void deleteEntry(entry.id)}
                  >
                    <Trash2 size={13} />
                  </button>
                )}
              </div>
            ))}
          </div>
          {agent === 'translation' && (
            <div className="skill-list-foot">
              <button type="button" className="chip" onClick={addPrompt}>
                <Plus size={13} />
                {t.set.addPrompt}
              </button>
            </div>
          )}
        </div>

        <div className="card">
          <div className="card-title">
            <FilePenLine size={18} />
            <span>{selectedSkill?.title ?? 'Skill'}</span>
          </div>
          <p className="card-desc">{selectedSkill?.description ?? ''}</p>
          <textarea
            value={skillDraft}
            onChange={(event) => setSkillDraft(event.target.value)}
            className="field-area"
            placeholder={t.set.skillPlaceholder}
            aria-label={selectedSkill?.title ?? 'Skill'}
          />
          <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: 12 }}>
            <button type="button" className="btn sm" onClick={() => void onSaveSkill()} disabled={skillSaveState === 'saving'}>
              {skillSaveState === 'saving' ? t.set.savingSkill : t.set.saveSkill}
            </button>
          </div>

          <div className="divider" />
          <div className="card-title" style={{ fontSize: 14 }}>
            <span>{t.set.promptPreview}</span>
            <span className="chip" style={{ fontSize: 11 }}>{preview?.skillIds.length ?? 0}</span>
          </div>
          <pre className="prompt-preview">{preview?.systemPrompt ?? ''}</pre>
        </div>
      </div>
    </div>
  );
}

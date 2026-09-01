import { Highlighter, Languages, Rows3 } from 'lucide-react';
import { useT } from '@/i18n';
import type { AppSettings } from '@/types';
import type { SettingsControlProps } from './panelTypes';

export function ReadingSettingsPanel({
  settings,
  updateSetting,
}: Pick<SettingsControlProps, 'settings' | 'updateSetting'>) {
  const t = useT();

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      <div className="row2">
        <div className="grow">
          <div className="name"><Languages size={15} style={{ color: 'var(--ink-3)' }} />{t.set.bilingualDefault}</div>
          <div className="desc">{t.set.bilingualNote}</div>
        </div>
        <button
          type="button"
          className={`switch ${settings.bilingualDefault ? 'on' : ''}`}
          onClick={() => updateSetting('bilingualDefault', !settings.bilingualDefault)}
          aria-pressed={settings.bilingualDefault}
          aria-label={t.set.bilingualDefault}
        />
      </div>

      <div className="row2">
        <div className="grow">
          <div className="name"><Highlighter size={15} style={{ color: 'var(--ink-3)' }} />{t.set.wordwiseDefault}</div>
          <div className="desc">{t.set.wordwiseNote}</div>
        </div>
        <button
          type="button"
          className={`switch ${settings.wordwiseDefault ? 'on' : ''}`}
          onClick={() => updateSetting('wordwiseDefault', !settings.wordwiseDefault)}
          aria-pressed={settings.wordwiseDefault}
          aria-label={t.set.wordwiseDefault}
        />
      </div>

      <div className="row2">
        <div className="grow">
          <div className="name"><Rows3 size={15} style={{ color: 'var(--ink-3)' }} />{t.set.density}</div>
          <div className="desc">{t.set.densityNote}</div>
        </div>
        <select
          value={settings.paragraphDensity}
          onChange={(event) =>
            updateSetting('paragraphDensity', event.target.value as AppSettings['paragraphDensity'])
          }
          className="v2select"
        >
          <option value="compact">{t.set.compact}</option>
          <option value="standard">{t.set.standard}</option>
          <option value="relaxed">{t.set.relaxed}</option>
        </select>
      </div>
    </div>
  );
}

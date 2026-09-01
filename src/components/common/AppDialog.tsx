import { createContext, useCallback, useContext, useRef, useState, type ReactNode } from 'react';
import { AnimatePresence, motion } from 'motion/react';

/**
 * 自绘确认/输入弹窗（禁止浏览器原生 prompt/confirm）。
 * Promise 风格调用：`await appConfirm({...})` / `await appPrompt({...})`，
 * 交互时长短、遮罩即取消——遵循「轻交互浮层」UX 原则，样式与设计语言一致。
 */

interface ConfirmOptions {
  title: string;
  message?: string;
  confirmLabel?: string;
  danger?: boolean;
}

interface PromptOptions {
  title: string;
  message?: string;
  placeholder?: string;
  initial?: string;
  confirmLabel?: string;
}

interface PendingDialog {
  id: number;
  kind: 'confirm' | 'prompt';
  confirm: Partial<ConfirmOptions> & Pick<ConfirmOptions, never>;
  prompt: Partial<PromptOptions>;
  resolveConfirm: (value: boolean) => void;
  resolvePrompt: (value: string | null) => void;
}

interface AppDialogContextType {
  appConfirm: (options: ConfirmOptions) => Promise<boolean>;
  appPrompt: (options: PromptOptions) => Promise<string | null>;
}

const AppDialogContext = createContext<AppDialogContextType | null>(null);

export function useAppDialog() {
  const context = useContext(AppDialogContext);
  if (!context) {
    throw new Error('useAppDialog must be used within an AppDialogProvider');
  }
  return context;
}

export function AppDialogProvider({ children }: { children: ReactNode }) {
  const [dialog, setDialog] = useState<PendingDialog | null>(null);
  const [draft, setDraft] = useState('');
  const idRef = useRef(0);

  const appConfirm = useCallback(
    (options: ConfirmOptions) =>
      new Promise<boolean>((resolve) => {
        idRef.current += 1;
        setDialog({
          id: idRef.current,
          kind: 'confirm',
          confirm: options,
          prompt: {},
          resolveConfirm: resolve,
          resolvePrompt: () => undefined,
        });
      }),
    [],
  );

  const appPrompt = useCallback(
    (options: PromptOptions) =>
      new Promise<string | null>((resolve) => {
        idRef.current += 1;
        setDraft(options.initial ?? '');
        setDialog({
          id: idRef.current,
          kind: 'prompt',
          confirm: {},
          prompt: options,
          resolveConfirm: () => undefined,
          resolvePrompt: resolve,
        });
      }),
    [],
  );

  const settle = (result: boolean | string | null) => {
    if (!dialog) return;
    if (dialog.kind === 'confirm') dialog.resolveConfirm(result as boolean);
    else dialog.resolvePrompt(result as string | null);
    setDialog(null);
  };

  const confirmActive = dialog?.kind === 'confirm';
  const submitPrompt = () => {
    if (!dialog || dialog.kind !== 'prompt') return;
    const value = draft.trim();
    if (!value) return;
    settle(value);
  };

  return (
    <AppDialogContext.Provider value={{ appConfirm, appPrompt }}>
      {children}
      <AnimatePresence>
        {dialog && (
          <motion.div
            key="app-dialog"
            className="fixed inset-0 z-[200]"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.18 }}
            role="dialog"
            aria-modal="true"
            aria-label={confirmActive ? dialog.confirm.title : dialog.prompt.title}
          >
            <button
              type="button"
              aria-label="close"
              className="absolute inset-0 bg-black/25"
              onClick={() => settle(confirmActive ? false : null)}
            />
            <motion.div
              initial={{ opacity: 0, scale: 0.96, y: 8 }}
              animate={{ opacity: 1, scale: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.97, y: 4 }}
              transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
              className="app-dialog-card"
              onKeyDown={(e) => {
                if (e.key === 'Escape') settle(confirmActive ? false : null);
                if (e.key === 'Enter' && !confirmActive) submitPrompt();
              }}
            >
              <h3 className="app-dialog-title">
                {confirmActive ? dialog.confirm.title : dialog.prompt.title}
              </h3>
              {(confirmActive ? dialog.confirm.message : dialog.prompt.message) && (
                <p className="app-dialog-message">{confirmActive ? dialog.confirm.message : dialog.prompt.message}</p>
              )}
              {!confirmActive && (
                <input
                  className="app-dialog-input"
                  autoFocus
                  value={draft}
                  placeholder={dialog.prompt.placeholder}
                  onChange={(e) => setDraft(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') submitPrompt();
                    if (e.key === 'Escape') settle(null);
                  }}
                />
              )}
              <div className="app-dialog-actions">
                <button type="button" className="btn ghost sm" onClick={() => settle(confirmActive ? false : null)}>
                  取消
                </button>
                {confirmActive ? (
                  <button
                    type="button"
                    className={`btn sm ${dialog.confirm.danger ? 'danger' : ''}`}
                    autoFocus
                    onClick={() => settle(true)}
                  >
                    {dialog.confirm.confirmLabel ?? '确定'}
                  </button>
                ) : (
                  <button type="button" className="btn sm" onClick={submitPrompt}>
                    {dialog.prompt.confirmLabel ?? '确定'}
                  </button>
                )}
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>
    </AppDialogContext.Provider>
  );
}

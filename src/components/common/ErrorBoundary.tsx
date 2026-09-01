import { Component, type ErrorInfo, type ReactNode } from 'react';

interface ErrorBoundaryProps {
  children: ReactNode;
}

interface ErrorBoundaryState {
  error: Error | null;
}

/**
 * 全局错误兜底：应用树任意子树抛出未捕获异常时，展示可恢复的降级界面，
 * 而不是白屏。放在 Providers 之外（main.tsx 顶层）——即使 Provider 自身
 * 抛错也能兜住。文案静态硬编码：崩溃可能发生在 i18n 上下文内部，
 * 不能依赖 useT。
 */
export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('[ErrorBoundary]', error, info.componentStack);
  }

  private handleReload = () => {
    window.location.reload();
  };

  private handleRetry = () => {
    this.setState({ error: null });
  };

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div
        style={{
          minHeight: '100vh',
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          gap: 16,
          padding: 24,
          fontFamily: 'system-ui, sans-serif',
          color: '#5a4a3a',
          background: '#f6f1e7',
        }}
      >
        <div style={{ fontSize: 18, fontWeight: 600 }}>界面出现了一个意外错误</div>
        <div style={{ fontSize: 13, color: '#8a7a66', maxWidth: 520, textAlign: 'center', wordBreak: 'break-word' }}>
          {error.message || String(error)}
        </div>
        <div style={{ display: 'flex', gap: 12, marginTop: 8 }}>
          <button
            type="button"
            onClick={this.handleRetry}
            style={{
              padding: '8px 18px',
              borderRadius: 8,
              border: '1px solid #c9b896',
              background: '#fffdf8',
              color: '#5a4a3a',
              cursor: 'pointer',
            }}
          >
            重试
          </button>
          <button
            type="button"
            onClick={this.handleReload}
            style={{
              padding: '8px 18px',
              borderRadius: 8,
              border: 'none',
              background: '#8a6d3b',
              color: '#fff',
              cursor: 'pointer',
            }}
          >
            重新加载应用
          </button>
        </div>
      </div>
    );
  }
}

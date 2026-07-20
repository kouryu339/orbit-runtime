import React from 'react';

type ErrorBoundaryState = {
  error: Error | null;
};

export class ErrorBoundary extends React.Component<React.PropsWithChildren, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error('Workflow Studio render failed', error, info);
  }

  render() {
    if (!this.state.error) return this.props.children;

    return (
      <main className="studio-fatal">
        <section>
          <h1>Workflow Studio recovered</h1>
          <p>{this.state.error.message}</p>
          <button type="button" onClick={() => location.reload()}>
            Reload Studio
          </button>
        </section>
      </main>
    );
  }
}

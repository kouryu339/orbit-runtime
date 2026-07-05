import React from 'react';
import ReactDOM from 'react-dom/client';
import '@xyflow/react/dist/style.css';
import { ErrorBoundary } from './ErrorBoundary';
import { StudioApp } from './StudioApp';
import './studio.css';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <StudioApp />
    </ErrorBoundary>
  </React.StrictMode>
);

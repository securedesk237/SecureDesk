import React from 'react';
import { appWindow } from '@tauri-apps/api/window';
import { FiMinus, FiSquare, FiX, FiShield, FiLock } from 'react-icons/fi';
import { SessionInfo } from '../App';
import './TitleBar.css';

interface TitleBarProps {
  session: SessionInfo | null;
}

const TitleBar: React.FC<TitleBarProps> = ({ session }) => {
  return (
    <div className="titlebar" data-tauri-drag-region>
      <div className="titlebar-left">
        <div className="titlebar-brand">
          <FiShield className="brand-icon" />
          <span className="brand-name">SecureDesk</span>
        </div>
      </div>

      <div className="titlebar-center" data-tauri-drag-region>
        {session && (
          <div className="session-badge">
            <FiLock className="session-icon" />
            <span>Connected to {session.remoteName}</span>
            <span className="encryption-tag">E2E</span>
          </div>
        )}
      </div>

      <div className="titlebar-right">
        <button
          className="titlebar-btn"
          onClick={() => appWindow.minimize()}
          title="Minimize"
        >
          <FiMinus />
        </button>
        <button
          className="titlebar-btn"
          onClick={() => appWindow.toggleMaximize()}
          title="Maximize"
        >
          <FiSquare />
        </button>
        <button
          className="titlebar-btn close"
          onClick={() => appWindow.hide()}
          title="Close to Tray"
        >
          <FiX />
        </button>
      </div>
    </div>
  );
};

export default TitleBar;

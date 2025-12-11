import React, { useState } from 'react';
import { FiCopy, FiCheck, FiMonitor, FiRefreshCw, FiSettings } from 'react-icons/fi';
import { DeviceInfo } from '../App';
import './ThisDevice.css';

interface ThisDeviceProps {
  device: DeviceInfo;
  isHosting: boolean;
  p2pEnabled: boolean;
  onRegenerateId?: () => Promise<void>;
  onOpenSettings?: () => void;
}

const ThisDevice: React.FC<ThisDeviceProps> = ({ device, isHosting, p2pEnabled, onRegenerateId, onOpenSettings }) => {
  const [copied, setCopied] = useState(false);
  const [regenerating, setRegenerating] = useState(false);

  const copyId = async () => {
    await navigator.clipboard.writeText(device.id.replace(/\s/g, ''));
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const handleRegenerate = async () => {
    if (regenerating || !onRegenerateId) return;
    setRegenerating(true);
    try {
      await onRegenerateId();
    } finally {
      setRegenerating(false);
    }
  };

  return (
    <div className="panel this-device">
      <div className="panel-header">
        <div className="panel-title">
          <FiMonitor className="panel-icon" />
          <span>This Device</span>
        </div>
        <div className="panel-subtitle">Share this ID to receive connections</div>
      </div>

      <div className="device-id-box">
        <div className="id-label">Your ID</div>
        <div className="id-row">
          <span className="id-value">{device.id}</span>
          <div className="id-actions">
            <button
              className={`id-btn ${copied ? 'copied' : ''}`}
              onClick={copyId}
              title="Copy ID"
            >
              {copied ? <FiCheck /> : <FiCopy />}
            </button>
            <button
              className={`id-btn ${regenerating ? 'spinning' : ''}`}
              onClick={handleRegenerate}
              title="Regenerate ID"
              disabled={regenerating}
            >
              <FiRefreshCw />
            </button>
          </div>
        </div>
      </div>

      <div className="status-row">
        <div className={`status-dot ${isHosting ? 'active' : 'ready'}`} />
        <span className="status-text">
          {isHosting ? 'Session active' : 'Ready to receive connections'}
        </span>
      </div>

      <div className="password-section">
        <div className="password-label">Session Password (optional)</div>
        <input
          type="password"
          className="password-input"
          placeholder="Set a password for extra security"
        />
      </div>

      <div className="settings-row">
        <div className="p2p-indicator">
          <span className={`p2p-badge ${p2pEnabled ? 'enabled' : 'disabled'}`}>
            {p2pEnabled ? '⚡ P2P Enabled' : '🔒 Relay Only'}
          </span>
        </div>
        <button className="settings-btn" onClick={onOpenSettings} title="Settings">
          <FiSettings />
        </button>
      </div>
    </div>
  );
};

export default ThisDevice;

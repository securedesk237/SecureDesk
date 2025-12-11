import React, { useState } from 'react';
import { motion } from 'framer-motion';
import { FiArrowRight, FiLoader, FiLink } from 'react-icons/fi';
import './RemoteConnect.css';

interface RemoteConnectProps {
  onConnect: (remoteId: string) => void;
  isConnecting: boolean;
}

const RemoteConnect: React.FC<RemoteConnectProps> = ({ onConnect, isConnecting }) => {
  const [remoteId, setRemoteId] = useState('');

  const formatId = (value: string) => {
    const digits = value.replace(/\D/g, '').slice(0, 9);
    const parts = [];
    for (let i = 0; i < digits.length; i += 3) {
      parts.push(digits.slice(i, i + 3));
    }
    return parts.join(' ');
  };

  const handleInputChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setRemoteId(formatId(e.target.value));
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (remoteId.replace(/\s/g, '').length === 9 && !isConnecting) {
      onConnect(remoteId);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      handleSubmit(e);
    }
  };

  const isValidId = remoteId.replace(/\s/g, '').length === 9;

  return (
    <div className="panel remote-connect">
      <div className="panel-header">
        <div className="panel-title">
          <FiLink className="panel-icon" />
          <span>Remote Desktop</span>
        </div>
        <div className="panel-subtitle">Enter the ID of the device to connect</div>
      </div>

      <form className="connect-form" onSubmit={handleSubmit}>
        <div className="input-wrapper">
          <label className="input-label">Remote ID</label>
          <input
            type="text"
            className="remote-id-input"
            placeholder="000 000 000"
            value={remoteId}
            onChange={handleInputChange}
            onKeyDown={handleKeyDown}
            disabled={isConnecting}
            autoComplete="off"
          />
        </div>

        <motion.button
          type="submit"
          className="connect-button"
          disabled={!isValidId || isConnecting}
          whileHover={{ scale: isValidId && !isConnecting ? 1.02 : 1 }}
          whileTap={{ scale: isValidId && !isConnecting ? 0.98 : 1 }}
        >
          {isConnecting ? (
            <>
              <FiLoader className="spin" />
              <span>Connecting...</span>
            </>
          ) : (
            <>
              <span>Connect</span>
              <FiArrowRight />
            </>
          )}
        </motion.button>
      </form>

      <div className="recent-section">
        <div className="recent-label">Recent Connections</div>
        <div className="recent-list">
          <div className="recent-empty">No recent connections</div>
        </div>
      </div>
    </div>
  );
};

export default RemoteConnect;

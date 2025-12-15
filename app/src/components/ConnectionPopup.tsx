import React, { useCallback, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { motion, AnimatePresence } from 'framer-motion';
import './ConnectionPopup.css';

interface ConnectionPopupProps {
  remoteId: string | null;
  onAccept: () => void;
  onDecline: () => void;
}

const ConnectionPopup: React.FC<ConnectionPopupProps> = ({ remoteId, onAccept, onDecline }) => {
  const [trustDevice, setTrustDevice] = useState(false);

  if (!remoteId) return null;

  // Format ID with spaces for display (XXX XXX XXX)
  const formatId = (id: string) => {
    const clean = id.replace(/\s/g, '');
    return clean.replace(/(.{3})(?=.)/g, '$1 ');
  };

  // Direct click handlers that stop propagation
  const handleAcceptClick = async (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    console.log('Accept button clicked, trust:', trustDevice);

    // If trust checkbox is checked, add to trusted devices
    if (trustDevice && remoteId) {
      try {
        await invoke('add_trusted_device', { deviceId: remoteId, name: null });
        console.log('Device added to trusted list:', remoteId);
      } catch (error) {
        console.error('Failed to add trusted device:', error);
      }
    }

    onAccept();
  };

  const handleDeclineClick = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    console.log('Decline button clicked');
    onDecline();
  }, [onDecline]);

  // Prevent overlay clicks from doing anything
  const handleOverlayClick = useCallback((e: React.MouseEvent) => {
    e.stopPropagation();
  }, []);

  return (
    <AnimatePresence>
      {remoteId && (
        <motion.div
          className="connection-popup-overlay"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          onClick={handleOverlayClick}
        >
          <motion.div
            className="connection-popup"
            initial={{ scale: 0.8, opacity: 0, y: -20 }}
            animate={{ scale: 1, opacity: 1, y: 0 }}
            exit={{ scale: 0.8, opacity: 0, y: -20 }}
            transition={{ type: 'spring', damping: 25, stiffness: 300 }}
            onClick={handleOverlayClick}
          >
            <div className="popup-icon">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4" />
                <polyline points="10 17 15 12 10 7" />
                <line x1="15" y1="12" x2="3" y2="12" />
              </svg>
            </div>
            <div className="popup-content">
              <h3>Incoming Connection Request</h3>
              <p className="popup-id">{formatId(remoteId)}</p>
              <p className="popup-subtitle">wants to connect to your device</p>
            </div>
            <div className="popup-warning">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
              </svg>
              <span>They will be able to see and control your screen</span>
            </div>

            {/* Trust device checkbox */}
            <label className="trust-checkbox">
              <input
                type="checkbox"
                checked={trustDevice}
                onChange={(e) => setTrustDevice(e.target.checked)}
              />
              <span className="checkmark"></span>
              <span className="trust-label">Trust this device (auto-accept future connections)</span>
            </label>

            <div className="popup-actions">
              <button
                type="button"
                className="popup-btn decline"
                onClick={handleDeclineClick}
              >
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                  <line x1="18" y1="6" x2="6" y2="18" />
                  <line x1="6" y1="6" x2="18" y2="18" />
                </svg>
                Decline
              </button>
              <button
                type="button"
                className="popup-btn accept"
                onClick={handleAcceptClick}
              >
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                  <polyline points="20 6 9 17 4 12" />
                </svg>
                Accept
              </button>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
};

export default ConnectionPopup;

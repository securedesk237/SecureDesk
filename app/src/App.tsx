import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/tauri';
import { listen } from '@tauri-apps/api/event';
import { motion, AnimatePresence } from 'framer-motion';
import TitleBar from './components/TitleBar';
import ThisDevice from './components/ThisDevice';
import RemoteConnect from './components/RemoteConnect';
import SessionView from './components/SessionView';
import ConnectionPopup from './components/ConnectionPopup';
import Settings from './components/Settings';
import './styles/app.css';

export type AppMode = 'idle' | 'hosting' | 'connecting' | 'connected';

export interface DeviceInfo {
  id: string;
  name: string;
}

export interface SessionInfo {
  remoteId: string;
  remoteName: string;
  isHost: boolean;
  startTime: Date;
}

interface ConnectionRequest {
  remote_id: string;
}

function App() {
  const [mode, setMode] = useState<AppMode>('idle');
  const [myDevice, setMyDevice] = useState<DeviceInfo>({ id: '--- --- ---', name: 'Loading...' });
  const [session, setSession] = useState<SessionInfo | null>(null);
  const [blackScreen, setBlackScreen] = useState(false);
  const [inputBlock, setInputBlock] = useState(false);
  const [incomingConnection, setIncomingConnection] = useState<string | null>(null);
  const [p2pEnabled, setP2pEnabled] = useState(true);
  const [connectionType, setConnectionType] = useState('None');
  const [settingsOpen, setSettingsOpen] = useState(false);

  useEffect(() => {
    // Get device ID from backend
    invoke<string>('get_device_id').then((id) => {
      setMyDevice({
        id: id,
        name: 'This Computer',
      });
    }).catch(console.error);

    // Get P2P enabled state
    invoke<boolean>('get_p2p_enabled').then((enabled) => {
      setP2pEnabled(enabled);
    }).catch(console.error);

    // Listen for incoming connections
    invoke('start_host_listener').catch(console.error);

    // Listen for connection request events from backend
    const unlistenRequest = listen<ConnectionRequest>('connection-request', async (event) => {
      console.log('Connection request from:', event.payload.remote_id);

      // Check if this device is trusted - auto-accept if so
      try {
        const isTrusted = await invoke<boolean>('is_device_trusted', { deviceId: event.payload.remote_id });
        if (isTrusted) {
          console.log('Auto-accepting trusted device:', event.payload.remote_id);
          await invoke('respond_to_connection', { accept: true });
          return;
        }
      } catch (error) {
        console.error('Failed to check trusted device:', error);
      }

      // Not trusted - show popup
      setIncomingConnection(event.payload.remote_id);
    });

    // Listen for connection accepted events
    const unlistenAccepted = listen<ConnectionRequest>('connection-accepted', (event) => {
      console.log('Connection accepted from:', event.payload.remote_id);
      setIncomingConnection(null);
      setMode('hosting');
    });

    // Listen for connection type changes
    const unlistenTypeChange = listen<{ type: string }>('connection-type-changed', (event) => {
      console.log('Connection type changed:', event.payload.type);
      setConnectionType(event.payload.type);
    });

    // Cleanup listeners on unmount
    return () => {
      unlistenRequest.then(fn => fn());
      unlistenAccepted.then(fn => fn());
      unlistenTypeChange.then(fn => fn());
    };
  }, []);

  const handleConnect = async (remoteId: string) => {
    setMode('connecting');
    try {
      await invoke('connect_to_remote', { remoteId: remoteId.replace(/\s/g, '') });
      setSession({
        remoteId,
        remoteName: `Remote-${remoteId.substring(0, 3)}`,
        isHost: false,
        startTime: new Date(),
      });
      setMode('connected');

      // Get connection type after connecting
      const connType = await invoke<string>('get_connection_type');
      setConnectionType(connType);
    } catch (error) {
      console.error('Connection failed:', error);
      setMode('idle');
    }
  };

  const handleP2PToggle = async (enabled: boolean) => {
    try {
      await invoke('set_p2p_enabled', { enabled });
      setP2pEnabled(enabled);
    } catch (error) {
      console.error('P2P toggle failed:', error);
    }
  };

  const handleDisconnect = async () => {
    try {
      await invoke('disconnect_session');
    } catch (error) {
      console.error('Disconnect error:', error);
    }
    setSession(null);
    setBlackScreen(false);
    setInputBlock(false);
    setMode('idle');
  };

  const handleToggleBlackScreen = async () => {
    const newValue = !blackScreen;
    try {
      await invoke('set_black_screen', { enabled: newValue });
      setBlackScreen(newValue);
    } catch (error) {
      console.error('Black screen toggle failed:', error);
    }
  };

  const handleToggleInputBlock = async () => {
    const newValue = !inputBlock;
    try {
      await invoke('set_input_block', { enabled: newValue });
      setInputBlock(newValue);
    } catch (error) {
      console.error('Input block toggle failed:', error);
    }
  };

  const handleRegenerateId = async () => {
    try {
      console.log('Regenerating ID...');
      const newId = await invoke<string>('regenerate_device_id');
      console.log('New ID:', newId);
      setMyDevice({
        id: newId,
        name: 'This Computer',
      });
      // Restart host listener with new identity
      invoke('start_host_listener').catch(console.error);
    } catch (error) {
      console.error('Regenerate ID failed:', error);
      alert('Failed to regenerate ID: ' + error);
    }
  };

  const handleAcceptConnection = async () => {
    if (!incomingConnection) return;
    try {
      console.log('Accepting connection from:', incomingConnection);
      await invoke('respond_to_connection', { accept: true });
      // The connection-accepted event will handle state updates
    } catch (error) {
      console.error('Accept connection failed:', error);
      setIncomingConnection(null);
    }
  };

  const handleDeclineConnection = async () => {
    if (!incomingConnection) return;
    try {
      console.log('Declining connection from:', incomingConnection);
      await invoke('respond_to_connection', { accept: false });
    } catch (error) {
      console.error('Decline connection failed:', error);
    }
    setIncomingConnection(null);
  };

  return (
    <div className="app">
      <TitleBar session={session} />

      {/* Connection Popup */}
      <ConnectionPopup
        remoteId={incomingConnection}
        onAccept={handleAcceptConnection}
        onDecline={handleDeclineConnection}
      />

      {/* Settings Panel */}
      <Settings
        p2pEnabled={p2pEnabled}
        onP2PToggle={handleP2PToggle}
        isOpen={settingsOpen}
        onClose={() => setSettingsOpen(false)}
      />

      <div className="app-body">
        <AnimatePresence mode="wait">
          {mode === 'connected' && session ? (
            <motion.div
              key="session"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              className="session-container"
            >
              <SessionView
                session={session}
                blackScreen={blackScreen}
                inputBlock={inputBlock}
                connectionType={connectionType}
                onToggleBlackScreen={handleToggleBlackScreen}
                onToggleInputBlock={handleToggleInputBlock}
                onDisconnect={handleDisconnect}
              />
            </motion.div>
          ) : (
            <motion.div
              key="main"
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -10 }}
              className="main-container"
            >
              <div className="main-content">
                <div className="panels-row">
                  {/* Left: This Device (Host mode) */}
                  <ThisDevice
                    device={myDevice}
                    isHosting={mode === 'hosting'}
                    p2pEnabled={p2pEnabled}
                    onRegenerateId={handleRegenerateId}
                    onOpenSettings={() => setSettingsOpen(true)}
                  />

                  {/* Right: Remote Connect (Client mode) */}
                  <RemoteConnect
                    onConnect={handleConnect}
                    isConnecting={mode === 'connecting'}
                  />
                </div>

                {/* Features */}
                <div className="features-row">
                  <div className="feature-card">
                    <div className="feature-icon">🔐</div>
                    <div className="feature-content">
                      <span className="feature-title">End-to-End Encrypted</span>
                      <span className="feature-desc">Military-grade encryption protects all sessions</span>
                    </div>
                  </div>
                  <div className="feature-card">
                    <div className="feature-icon">👁️‍🗨️</div>
                    <div className="feature-content">
                      <span className="feature-title">Zero Telemetry</span>
                      <span className="feature-desc">No data collection, no tracking, ever</span>
                    </div>
                  </div>
                  <div className="feature-card">
                    <div className="feature-icon">🖥️</div>
                    <div className="feature-content">
                      <span className="feature-title">Privacy Mode</span>
                      <span className="feature-desc">Black screen hides sensitive operations</span>
                    </div>
                  </div>
                </div>
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </div>
  );
}

export default App;

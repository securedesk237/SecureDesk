import React, { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { motion, AnimatePresence } from 'framer-motion';
import {
  FiMaximize,
  FiMinimize,
  FiMonitor,
  FiMousePointer,
  FiEye,
  FiEyeOff,
  FiLock,
  FiUnlock,
  FiClipboard,
  FiFolder,
  FiSettings,
  FiX,
  FiZoomIn,
  FiZoomOut,
  FiRefreshCw,
  FiCheck,
  FiCopy,
  FiDownload,
  FiUpload,
  FiCircle,
  FiSquare,
} from 'react-icons/fi';
import { SessionInfo } from '../App';
import './SessionView.css';

interface VideoFrame {
  width: number;
  height: number;
  data: string; // Base64 encoded
}

interface SessionViewProps {
  session: SessionInfo;
  blackScreen: boolean;
  inputBlock: boolean;
  connectionType: string;
  onToggleBlackScreen: () => void;
  onToggleInputBlock: () => void;
  onDisconnect: () => void;
}

// Map browser key codes to Windows virtual key codes
const keyCodeToVK: { [key: string]: number } = {
  'Backspace': 0x08, 'Tab': 0x09, 'Enter': 0x0D, 'ShiftLeft': 0x10, 'ShiftRight': 0x10,
  'ControlLeft': 0x11, 'ControlRight': 0x11, 'AltLeft': 0x12, 'AltRight': 0x12,
  'Pause': 0x13, 'CapsLock': 0x14, 'Escape': 0x1B, 'Space': 0x20,
  'PageUp': 0x21, 'PageDown': 0x22, 'End': 0x23, 'Home': 0x24,
  'ArrowLeft': 0x25, 'ArrowUp': 0x26, 'ArrowRight': 0x27, 'ArrowDown': 0x28,
  'PrintScreen': 0x2C, 'Insert': 0x2D, 'Delete': 0x2E,
  'Digit0': 0x30, 'Digit1': 0x31, 'Digit2': 0x32, 'Digit3': 0x33, 'Digit4': 0x34,
  'Digit5': 0x35, 'Digit6': 0x36, 'Digit7': 0x37, 'Digit8': 0x38, 'Digit9': 0x39,
  'KeyA': 0x41, 'KeyB': 0x42, 'KeyC': 0x43, 'KeyD': 0x44, 'KeyE': 0x45,
  'KeyF': 0x46, 'KeyG': 0x47, 'KeyH': 0x48, 'KeyI': 0x49, 'KeyJ': 0x4A,
  'KeyK': 0x4B, 'KeyL': 0x4C, 'KeyM': 0x4D, 'KeyN': 0x4E, 'KeyO': 0x4F,
  'KeyP': 0x50, 'KeyQ': 0x51, 'KeyR': 0x52, 'KeyS': 0x53, 'KeyT': 0x54,
  'KeyU': 0x55, 'KeyV': 0x56, 'KeyW': 0x57, 'KeyX': 0x58, 'KeyY': 0x59, 'KeyZ': 0x5A,
  'MetaLeft': 0x5B, 'MetaRight': 0x5C,
  'Numpad0': 0x60, 'Numpad1': 0x61, 'Numpad2': 0x62, 'Numpad3': 0x63, 'Numpad4': 0x64,
  'Numpad5': 0x65, 'Numpad6': 0x66, 'Numpad7': 0x67, 'Numpad8': 0x68, 'Numpad9': 0x69,
  'NumpadMultiply': 0x6A, 'NumpadAdd': 0x6B, 'NumpadSubtract': 0x6D, 'NumpadDecimal': 0x6E, 'NumpadDivide': 0x6F,
  'F1': 0x70, 'F2': 0x71, 'F3': 0x72, 'F4': 0x73, 'F5': 0x74, 'F6': 0x75,
  'F7': 0x76, 'F8': 0x77, 'F9': 0x78, 'F10': 0x79, 'F11': 0x7A, 'F12': 0x7B,
  'NumLock': 0x90, 'ScrollLock': 0x91,
  'Semicolon': 0xBA, 'Equal': 0xBB, 'Comma': 0xBC, 'Minus': 0xBD, 'Period': 0xBE,
  'Slash': 0xBF, 'Backquote': 0xC0, 'BracketLeft': 0xDB, 'Backslash': 0xDC,
  'BracketRight': 0xDD, 'Quote': 0xDE,
};

interface ClipboardContent {
  data_type: string;
  text?: string;
  image_data?: string;
  files?: string[];
}

const SessionView: React.FC<SessionViewProps> = ({
  session,
  blackScreen,
  inputBlock,
  connectionType,
  onToggleBlackScreen,
  onToggleInputBlock,
  onDisconnect,
}) => {
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [controlMode, setControlMode] = useState(true);
  const [zoom, setZoom] = useState(100);
  const [duration, setDuration] = useState('00:00:00');
  const [frameData, setFrameData] = useState<string | null>(null);
  const [frameSize, setFrameSize] = useState({ width: 1920, height: 1080 });
  const [fps, setFps] = useState(0);
  const [latency, setLatency] = useState(0);
  const [showClipboardPanel, setShowClipboardPanel] = useState(false);
  const [clipboardSyncEnabled, setClipboardSyncEnabled] = useState(true);
  const [clipboardStatus, setClipboardStatus] = useState<string | null>(null);
  const [localClipboard, setLocalClipboard] = useState<ClipboardContent | null>(null);
  const [isRecording, setIsRecording] = useState(false);
  const [recordingDuration, setRecordingDuration] = useState('00:00');
  const viewportRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const imageRef = useRef<HTMLImageElement>(null);
  const frameCountRef = useRef(0);
  const lastFpsTimeRef = useRef(Date.now());

  // Send viewport resolution to host for adaptive scaling
  const sendViewportResolution = useCallback(async () => {
    if (!viewportRef.current) return;

    const rect = viewportRef.current.getBoundingClientRect();
    const width = Math.round(rect.width);
    const height = Math.round(rect.height - 100); // Account for toolbar/statusbar

    try {
      await invoke('send_resolution', { width, height });
      console.log(`Sent resolution: ${width}x${height}`);
    } catch (error) {
      console.error('Resolution send error:', error);
    }
  }, []);

  // Send resolution on mount and window resize
  useEffect(() => {
    // Delay to ensure viewport is rendered
    const timer = setTimeout(sendViewportResolution, 500);

    const handleResize = () => {
      sendViewportResolution();
    };

    window.addEventListener('resize', handleResize);
    return () => {
      clearTimeout(timer);
      window.removeEventListener('resize', handleResize);
    };
  }, [sendViewportResolution]);

  // Request video frames continuously
  useEffect(() => {
    let running = true;
    let consecutiveErrors = 0;

    const fetchFrame = async () => {
      if (!running) return;

      const startTime = Date.now();
      try {
        const frame = await invoke<VideoFrame | null>('request_video_frame');
        if (frame && running) {
          setFrameData(frame.data);
          setFrameSize({ width: frame.width, height: frame.height });
          setLatency(Date.now() - startTime);
          consecutiveErrors = 0;

          // Count FPS
          frameCountRef.current++;
          const now = Date.now();
          if (now - lastFpsTimeRef.current >= 1000) {
            setFps(frameCountRef.current);
            frameCountRef.current = 0;
            lastFpsTimeRef.current = now;
          }
        }
      } catch (error) {
        console.error('Frame request error:', error);
        consecutiveErrors++;
        // If too many consecutive errors, slow down requests
        if (consecutiveErrors > 5) {
          await new Promise(resolve => setTimeout(resolve, 500));
        }
      }

      // Request next frame (target ~30 FPS)
      if (running) {
        setTimeout(fetchFrame, 33);
      }
    };

    fetchFrame();

    return () => {
      running = false;
    };
  }, []);

  // Duration timer
  useEffect(() => {
    const timer = setInterval(() => {
      const diff = Math.floor((Date.now() - session.startTime.getTime()) / 1000);
      const h = Math.floor(diff / 3600).toString().padStart(2, '0');
      const m = Math.floor((diff % 3600) / 60).toString().padStart(2, '0');
      const s = (diff % 60).toString().padStart(2, '0');
      setDuration(`${h}:${m}:${s}`);
    }, 1000);
    return () => clearInterval(timer);
  }, [session.startTime]);

  // Handle mouse events for remote control
  const handleMouseEvent = useCallback(async (
    e: React.MouseEvent<HTMLElement>,
    eventType: 'move' | 'down' | 'up'
  ) => {
    if (!controlMode) return;

    const target = imageRef.current || canvasRef.current;
    if (!target) return;

    const rect = target.getBoundingClientRect();
    const scaleX = frameSize.width / rect.width;
    const scaleY = frameSize.height / rect.height;
    const x = Math.round((e.clientX - rect.left) * scaleX);
    const y = Math.round((e.clientY - rect.top) * scaleY);

    let button: number | undefined;
    if (eventType !== 'move') {
      button = e.button; // 0=left, 1=middle, 2=right
    }

    try {
      await invoke('send_mouse', { x, y, eventType, button });
    } catch (error) {
      console.error('Mouse event error:', error);
    }
  }, [controlMode, frameSize]);

  // Handle mouse scroll for remote control
  const handleWheel = useCallback(async (e: React.WheelEvent<HTMLElement>) => {
    if (!controlMode) return;
    e.preventDefault();

    // Normalize scroll delta (different browsers report different values)
    const deltaY = Math.sign(e.deltaY) * -1; // Invert for natural scrolling
    const deltaX = Math.sign(e.deltaX);

    try {
      await invoke('send_mouse', { x: deltaX, y: deltaY, eventType: 'scroll', button: null });
    } catch (error) {
      console.error('Scroll event error:', error);
    }
  }, [controlMode]);

  // Handle keyboard events for remote control
  const handleKeyEvent = useCallback(async (e: KeyboardEvent, pressed: boolean) => {
    if (!controlMode) return;

    // Prevent default for most keys to avoid browser shortcuts
    if (e.code !== 'F5' && e.code !== 'F12') {
      e.preventDefault();
    }

    const vk = keyCodeToVK[e.code];
    if (vk !== undefined) {
      try {
        await invoke('send_key', { keyCode: vk, pressed });
      } catch (error) {
        console.error('Key event error:', error);
      }
    }
  }, [controlMode]);

  // Set up keyboard event listeners
  useEffect(() => {
    if (!controlMode) return;

    const handleKeyDown = (e: KeyboardEvent) => handleKeyEvent(e, true);
    const handleKeyUp = (e: KeyboardEvent) => handleKeyEvent(e, false);

    // Use window to capture all keyboard events
    window.addEventListener('keydown', handleKeyDown);
    window.addEventListener('keyup', handleKeyUp);

    return () => {
      window.removeEventListener('keydown', handleKeyDown);
      window.removeEventListener('keyup', handleKeyUp);
    };
  }, [controlMode, handleKeyEvent]);

  const toggleFullscreen = async () => {
    if (!document.fullscreenElement) {
      await viewportRef.current?.requestFullscreen();
      setIsFullscreen(true);
    } else {
      await document.exitFullscreen();
      setIsFullscreen(false);
    }
  };

  // Clipboard functions
  const refreshLocalClipboard = useCallback(async () => {
    try {
      const content = await invoke<ClipboardContent | null>('get_local_clipboard');
      setLocalClipboard(content);
    } catch (error) {
      console.error('Failed to get clipboard:', error);
    }
  }, []);

  const sendClipboardToRemote = useCallback(async () => {
    try {
      setClipboardStatus('Sending...');
      await invoke('send_clipboard_to_remote');
      setClipboardStatus('Sent!');
      setTimeout(() => setClipboardStatus(null), 2000);
    } catch (error) {
      console.error('Failed to send clipboard:', error);
      setClipboardStatus('Failed');
      setTimeout(() => setClipboardStatus(null), 2000);
    }
  }, []);

  const requestRemoteClipboard = useCallback(async () => {
    try {
      setClipboardStatus('Requesting...');
      await invoke('request_remote_clipboard');
      setClipboardStatus('Received!');
      setTimeout(() => setClipboardStatus(null), 2000);
    } catch (error) {
      console.error('Failed to request clipboard:', error);
      setClipboardStatus('Failed');
      setTimeout(() => setClipboardStatus(null), 2000);
    }
  }, []);

  const toggleClipboardSync = useCallback(async () => {
    const newState = !clipboardSyncEnabled;
    setClipboardSyncEnabled(newState);
    try {
      await invoke('set_clipboard_sync_enabled', { enabled: newState });
    } catch (error) {
      console.error('Failed to toggle clipboard sync:', error);
    }
  }, [clipboardSyncEnabled]);

  // Listen for clipboard events
  useEffect(() => {
    const unlistenClipboard = listen('clipboard-received', () => {
      setClipboardStatus('Clipboard updated!');
      setTimeout(() => setClipboardStatus(null), 2000);
      refreshLocalClipboard();
    });

    // Initial clipboard sync state
    invoke<boolean>('get_clipboard_sync_enabled').then(setClipboardSyncEnabled).catch(console.error);

    return () => {
      unlistenClipboard.then(fn => fn());
    };
  }, [refreshLocalClipboard]);

  // Recording functions
  const toggleRecording = useCallback(async () => {
    try {
      if (isRecording) {
        await invoke('stop_recording');
        setIsRecording(false);
        setRecordingDuration('00:00');
      } else {
        await invoke('start_recording', {
          remoteDeviceId: session.remoteId,
          remoteDeviceName: session.remoteName,
        });
        setIsRecording(true);
      }
    } catch (error) {
      console.error('Recording error:', error);
    }
  }, [isRecording, session.remoteId, session.remoteName]);

  // Update recording duration
  useEffect(() => {
    if (!isRecording) return;

    const startTime = Date.now();
    const interval = setInterval(() => {
      const elapsed = Math.floor((Date.now() - startTime) / 1000);
      const mins = Math.floor(elapsed / 60).toString().padStart(2, '0');
      const secs = (elapsed % 60).toString().padStart(2, '0');
      setRecordingDuration(`${mins}:${secs}`);
    }, 1000);

    return () => clearInterval(interval);
  }, [isRecording]);

  // Check initial recording state
  useEffect(() => {
    invoke<boolean>('is_recording').then(setIsRecording).catch(console.error);
  }, []);

  return (
    <div className="session-view" ref={viewportRef}>
      {/* Toolbar */}
      <div className="session-toolbar">
        <div className="toolbar-section left">
          <div className="session-meta">
            <span className="meta-name">{session.remoteName}</span>
            <span className="meta-duration">{duration}</span>
          </div>
        </div>

        <div className="toolbar-section center">
          {/* Mode Toggle */}
          <div className="toolbar-group">
            <button
              className={`toolbar-btn ${controlMode ? 'active' : ''}`}
              onClick={() => setControlMode(true)}
            >
              <FiMousePointer />
              <span>Control</span>
            </button>
            <button
              className={`toolbar-btn ${!controlMode ? 'active' : ''}`}
              onClick={() => setControlMode(false)}
            >
              <FiEye />
              <span>View</span>
            </button>
          </div>

          <div className="toolbar-sep" />

          {/* Privacy */}
          <div className="toolbar-group">
            <button
              className={`toolbar-btn privacy ${blackScreen ? 'warning' : ''}`}
              onClick={onToggleBlackScreen}
              title="Black Screen"
            >
              {blackScreen ? <FiEyeOff /> : <FiMonitor />}
              <span>Black Screen</span>
            </button>
            <button
              className={`toolbar-btn privacy ${inputBlock ? 'warning' : ''}`}
              onClick={onToggleInputBlock}
              title="Block Input"
            >
              {inputBlock ? <FiLock /> : <FiUnlock />}
              <span>Block Input</span>
            </button>
          </div>

          <div className="toolbar-sep" />

          {/* Tools */}
          <div className="toolbar-group">
            <button
              className={`toolbar-btn icon-only ${isRecording ? 'recording' : ''}`}
              title={isRecording ? `Recording ${recordingDuration}` : 'Start Recording'}
              onClick={toggleRecording}
            >
              {isRecording ? <FiSquare /> : <FiCircle />}
            </button>
            <button
              className={`toolbar-btn icon-only ${showClipboardPanel ? 'active' : ''}`}
              title="Clipboard"
              onClick={() => {
                setShowClipboardPanel(!showClipboardPanel);
                if (!showClipboardPanel) refreshLocalClipboard();
              }}
            >
              <FiClipboard />
            </button>
            <button className="toolbar-btn icon-only" title="File Transfer">
              <FiFolder />
            </button>
            <button className="toolbar-btn icon-only" title="Settings">
              <FiSettings />
            </button>
          </div>

          <div className="toolbar-sep" />

          {/* Zoom */}
          <div className="toolbar-group zoom-group">
            <button
              className="toolbar-btn icon-only"
              onClick={() => setZoom(Math.max(25, zoom - 25))}
            >
              <FiZoomOut />
            </button>
            <span className="zoom-value">{zoom}%</span>
            <button
              className="toolbar-btn icon-only"
              onClick={() => setZoom(Math.min(200, zoom + 25))}
            >
              <FiZoomIn />
            </button>
            <button
              className="toolbar-btn icon-only"
              onClick={() => setZoom(100)}
              title="Reset"
            >
              <FiRefreshCw />
            </button>
            <button
              className="toolbar-btn icon-only"
              onClick={toggleFullscreen}
              title="Fullscreen"
            >
              {isFullscreen ? <FiMinimize /> : <FiMaximize />}
            </button>
          </div>
        </div>

        <div className="toolbar-section right">
          <motion.button
            className="disconnect-btn"
            onClick={onDisconnect}
            whileHover={{ scale: 1.03 }}
            whileTap={{ scale: 0.97 }}
          >
            <FiX />
            <span>End Session</span>
          </motion.button>
        </div>
      </div>

      {/* Viewport */}
      <div className="session-canvas">
        <div
          className="canvas-content"
          style={{ transform: `scale(${zoom / 100})` }}
        >
          {frameData ? (
            <img
              ref={imageRef}
              src={`data:image/jpeg;base64,${frameData}`}
              alt="Remote Desktop"
              className="remote-frame"
              tabIndex={0}
              style={{
                width: '100%',
                height: '100%',
                objectFit: 'contain',
                cursor: controlMode ? 'none' : 'default',
                outline: 'none',
              }}
              onMouseMove={(e) => handleMouseEvent(e, 'move')}
              onMouseDown={(e) => handleMouseEvent(e, 'down')}
              onMouseUp={(e) => handleMouseEvent(e, 'up')}
              onWheel={handleWheel}
              onContextMenu={(e) => e.preventDefault()}
              draggable={false}
            />
          ) : (
            <div className="canvas-placeholder">
              <FiMonitor className="placeholder-icon" />
              <span className="placeholder-title">Remote Desktop</span>
              <span className="placeholder-text">
                Connecting to video stream...
              </span>
            </div>
          )}
        </div>

        {/* Clipboard Panel */}
        <AnimatePresence>
          {showClipboardPanel && (
            <motion.div
              className="clipboard-panel"
              initial={{ opacity: 0, x: 20 }}
              animate={{ opacity: 1, x: 0 }}
              exit={{ opacity: 0, x: 20 }}
            >
              <div className="clipboard-panel-header">
                <h3>Clipboard Sync</h3>
                <button
                  className="clipboard-close"
                  onClick={() => setShowClipboardPanel(false)}
                >
                  <FiX />
                </button>
              </div>

              <div className="clipboard-panel-content">
                <div className="clipboard-sync-toggle">
                  <label>
                    <input
                      type="checkbox"
                      checked={clipboardSyncEnabled}
                      onChange={toggleClipboardSync}
                    />
                    <span>Auto-sync clipboard</span>
                  </label>
                </div>

                <div className="clipboard-actions">
                  <button
                    className="clipboard-action-btn"
                    onClick={sendClipboardToRemote}
                    title="Send your clipboard to remote"
                  >
                    <FiUpload />
                    <span>Send to Remote</span>
                  </button>
                  <button
                    className="clipboard-action-btn"
                    onClick={requestRemoteClipboard}
                    title="Get clipboard from remote"
                  >
                    <FiDownload />
                    <span>Get from Remote</span>
                  </button>
                </div>

                {clipboardStatus && (
                  <div className="clipboard-status">
                    <FiCheck />
                    <span>{clipboardStatus}</span>
                  </div>
                )}

                <div className="clipboard-preview">
                  <h4>Local Clipboard</h4>
                  {localClipboard ? (
                    <div className="clipboard-content">
                      {localClipboard.data_type === 'text' && (
                        <pre className="clipboard-text">
                          {localClipboard.text?.substring(0, 500)}
                          {(localClipboard.text?.length || 0) > 500 && '...'}
                        </pre>
                      )}
                      {localClipboard.data_type === 'image' && (
                        <div className="clipboard-image">
                          <FiCopy /> Image in clipboard
                        </div>
                      )}
                      {localClipboard.data_type === 'files' && (
                        <div className="clipboard-files">
                          <FiFolder /> {localClipboard.files?.length} file(s)
                        </div>
                      )}
                    </div>
                  ) : (
                    <div className="clipboard-empty">Clipboard is empty</div>
                  )}
                  <button
                    className="clipboard-refresh"
                    onClick={refreshLocalClipboard}
                  >
                    <FiRefreshCw /> Refresh
                  </button>
                </div>
              </div>
            </motion.div>
          )}
        </AnimatePresence>

        {/* Privacy Indicator - shows when black screen is active on remote */}
        {blackScreen && (
          <motion.div
            className="privacy-indicator"
            initial={{ opacity: 0, y: -10 }}
            animate={{ opacity: 1, y: 0 }}
          >
            <FiEyeOff className="privacy-indicator-icon" />
            <span>Privacy Mode Active - Remote screen is hidden</span>
          </motion.div>
        )}
      </div>

      {/* Status Bar */}
      <div className="session-status">
        <div className="status-left">
          <span className="status-item">
            <span className={`status-dot ${frameData ? 'connected' : 'connecting'}`} />
            {frameData ? 'Streaming' : 'Connecting...'}
          </span>
          <span className={`status-item connection-badge ${connectionType === 'P2P' ? 'p2p' : 'relay'}`}>
            {connectionType === 'P2P' ? '⚡ P2P Direct' : '🔒 Relay'}
          </span>
          <span className="status-item">
            <FiLock className="status-lock" />
            E2E Encrypted
          </span>
        </div>
        <div className="status-right">
          <span className="status-item">{frameSize.width}×{frameSize.height}</span>
          <span className="status-item">{fps} FPS</span>
          <span className="status-item">{latency}ms</span>
        </div>
      </div>
    </div>
  );
};

export default SessionView;

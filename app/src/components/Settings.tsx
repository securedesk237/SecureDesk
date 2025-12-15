import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-shell';
import './Settings.css';

interface SettingsProps {
  p2pEnabled: boolean;
  onP2PToggle: (enabled: boolean) => void;
  isOpen: boolean;
  onClose: () => void;
}

interface LicenseInfo {
  tier: string;
  key_id: string | null;
  expires_at: number | null;
  days_remaining: number | null;
  max_sessions: number;
  is_valid: boolean;
}

interface TrustedDevice {
  device_id: string;
  name: string | null;
  trusted_at: number;
  last_connected: number | null;
}

interface AppSettings {
  start_with_windows: boolean;
  minimize_to_tray: boolean;
  show_notifications: boolean;
  p2p_enabled: boolean;
  connection_quality: string;
  require_approval: boolean;
  lock_on_disconnect: boolean;
  session_timeout: number;
  hide_from_address_book: boolean;
}

type SettingsCategory =
  | 'general'
  | 'connection'
  | 'security'
  | 'privacy'
  | 'license'
  | 'trusted'
  | 'about';

function Settings({ p2pEnabled, onP2PToggle, isOpen, onClose }: SettingsProps) {
  const [activeCategory, setActiveCategory] = useState<SettingsCategory>('general');
  const [licenseKey, setLicenseKey] = useState('');
  const [licenseInfo, setLicenseInfo] = useState<LicenseInfo | null>(null);
  const [licenseError, setLicenseError] = useState('');
  const [licenseSuccess, setLicenseSuccess] = useState('');
  const [trustedDevices, setTrustedDevices] = useState<TrustedDevice[]>([]);
  const [settings, setSettings] = useState<AppSettings | null>(null);

  useEffect(() => {
    if (isOpen) {
      loadSettings();
      loadLicenseInfo();
      loadTrustedDevices();
    }
  }, [isOpen]);

  const loadSettings = async () => {
    try {
      const s = await invoke<AppSettings>('get_settings');
      setSettings(s);
    } catch (error) {
      console.error('Failed to load settings:', error);
    }
  };

  const loadLicenseInfo = async () => {
    try {
      const info = await invoke<LicenseInfo>('get_license_info');
      setLicenseInfo(info);
    } catch (error) {
      console.error('Failed to load license info:', error);
    }
  };

  const loadTrustedDevices = async () => {
    try {
      const devices = await invoke<TrustedDevice[]>('get_trusted_devices');
      setTrustedDevices(devices);
    } catch (error) {
      console.error('Failed to load trusted devices:', error);
    }
  };

  const updateBoolSetting = async (key: string, value: boolean) => {
    try {
      await invoke('set_setting_bool', { key, value });
      setSettings(prev => prev ? { ...prev, [key]: value } : null);

      // Keep P2P in sync with parent component
      if (key === 'p2p_enabled') {
        onP2PToggle(value);
      }
    } catch (error) {
      console.error(`Failed to update ${key}:`, error);
    }
  };

  const updateStringSetting = async (key: string, value: string) => {
    try {
      await invoke('set_setting_string', { key, value });
      setSettings(prev => prev ? { ...prev, [key]: value } : null);
    } catch (error) {
      console.error(`Failed to update ${key}:`, error);
    }
  };

  const updateNumberSetting = async (key: string, value: number) => {
    try {
      await invoke('set_setting_number', { key, value });
      setSettings(prev => prev ? { ...prev, [key]: value } : null);
    } catch (error) {
      console.error(`Failed to update ${key}:`, error);
    }
  };

  const handleActivateLicense = async () => {
    setLicenseError('');
    setLicenseSuccess('');

    if (!licenseKey.trim()) {
      setLicenseError('Please enter a license key');
      return;
    }

    try {
      const tier = await invoke<string>('activate_license', { licenseKey: licenseKey.trim() });
      setLicenseSuccess(`License activated! You now have ${tier} tier.`);
      setLicenseKey('');
      loadLicenseInfo();
    } catch (error) {
      setLicenseError(String(error));
    }
  };

  const handleDeactivateLicense = async () => {
    try {
      await invoke('deactivate_license');
      setLicenseSuccess('License deactivated. You are now on Free tier.');
      loadLicenseInfo();
    } catch (error) {
      setLicenseError(String(error));
    }
  };

  const handleRemoveTrustedDevice = async (deviceId: string) => {
    try {
      await invoke('remove_trusted_device', { deviceId });
      loadTrustedDevices();
    } catch (error) {
      console.error('Failed to remove trusted device:', error);
    }
  };

  const formatDeviceId = (id: string) => {
    const clean = id.replace(/\s/g, '');
    return clean.replace(/(.{3})(?=.)/g, '$1 ');
  };

  const formatDate = (timestamp: number) => {
    return new Date(timestamp * 1000).toLocaleDateString();
  };

  if (!isOpen) return null;

  const categories: { id: SettingsCategory; label: string; icon: string }[] = [
    { id: 'general', label: 'General', icon: '⚙️' },
    { id: 'connection', label: 'Connection', icon: '🔗' },
    { id: 'security', label: 'Security', icon: '🔒' },
    { id: 'privacy', label: 'Privacy', icon: '👁️' },
    { id: 'license', label: 'License', icon: '🔑' },
    { id: 'trusted', label: 'Trusted Devices', icon: '✓' },
    { id: 'about', label: 'About', icon: 'ℹ️' },
  ];

  const renderContent = () => {
    switch (activeCategory) {
      case 'general':
        return (
          <div className="settings-category-content">
            <h2>General Settings</h2>
            <div className="settings-item">
              <div className="settings-item-info">
                <span className="settings-item-label">Start with Windows</span>
                <span className="settings-item-desc">
                  Automatically start SecureDesk when Windows starts
                </span>
              </div>
              <label className="toggle-switch">
                <input
                  type="checkbox"
                  checked={settings?.start_with_windows ?? false}
                  onChange={(e) => updateBoolSetting('start_with_windows', e.target.checked)}
                />
                <span className="toggle-slider"></span>
              </label>
            </div>
            <div className="settings-item">
              <div className="settings-item-info">
                <span className="settings-item-label">Minimize to tray on close</span>
                <span className="settings-item-desc">
                  Keep SecureDesk running in the background when closing the window
                </span>
              </div>
              <label className="toggle-switch">
                <input
                  type="checkbox"
                  checked={settings?.minimize_to_tray ?? true}
                  onChange={(e) => updateBoolSetting('minimize_to_tray', e.target.checked)}
                />
                <span className="toggle-slider"></span>
              </label>
            </div>
            <div className="settings-item">
              <div className="settings-item-info">
                <span className="settings-item-label">Show notifications</span>
                <span className="settings-item-desc">
                  Display notifications for incoming connections
                </span>
              </div>
              <label className="toggle-switch">
                <input
                  type="checkbox"
                  checked={settings?.show_notifications ?? true}
                  onChange={(e) => updateBoolSetting('show_notifications', e.target.checked)}
                />
                <span className="toggle-slider"></span>
              </label>
            </div>
          </div>
        );

      case 'connection':
        return (
          <div className="settings-category-content">
            <h2>Connection Settings</h2>
            <div className="settings-item">
              <div className="settings-item-info">
                <span className="settings-item-label">P2P Direct Connection</span>
                <span className="settings-item-desc">
                  Enable peer-to-peer connections for faster speed. Your IP address will be visible to the remote device.
                </span>
              </div>
              <label className="toggle-switch">
                <input
                  type="checkbox"
                  checked={settings?.p2p_enabled ?? true}
                  onChange={(e) => updateBoolSetting('p2p_enabled', e.target.checked)}
                />
                <span className="toggle-slider"></span>
              </label>
            </div>
            <div className="settings-item">
              <div className="settings-item-info">
                <span className="settings-item-label">Connection quality</span>
                <span className="settings-item-desc">
                  Balance between speed and visual quality
                </span>
              </div>
              <select
                className="settings-select"
                value={settings?.connection_quality ?? 'auto'}
                onChange={(e) => updateStringSetting('connection_quality', e.target.value)}
              >
                <option value="auto">Auto (Recommended)</option>
                <option value="quality">Best Quality</option>
                <option value="balanced">Balanced</option>
                <option value="speed">Best Speed</option>
              </select>
            </div>
            <div className="settings-info-box">
              <p>
                <strong>P2P Enabled:</strong> Connections are established directly between devices when possible, providing lower latency. Falls back to relay if direct connection fails.
              </p>
              <p>
                <strong>P2P Disabled:</strong> All connections go through our encrypted relay servers, providing maximum privacy but slightly higher latency.
              </p>
            </div>
          </div>
        );

      case 'security':
        return (
          <div className="settings-category-content">
            <h2>Security Settings</h2>
            <div className="settings-item">
              <div className="settings-item-info">
                <span className="settings-item-label">Require approval for connections</span>
                <span className="settings-item-desc">
                  Always ask before accepting incoming connections (except trusted devices)
                </span>
              </div>
              <label className="toggle-switch">
                <input
                  type="checkbox"
                  checked={settings?.require_approval ?? true}
                  onChange={(e) => updateBoolSetting('require_approval', e.target.checked)}
                />
                <span className="toggle-slider"></span>
              </label>
            </div>
            <div className="settings-item">
              <div className="settings-item-info">
                <span className="settings-item-label">Lock screen on disconnect</span>
                <span className="settings-item-desc">
                  Automatically lock this computer when a remote session ends
                </span>
              </div>
              <label className="toggle-switch">
                <input
                  type="checkbox"
                  checked={settings?.lock_on_disconnect ?? false}
                  onChange={(e) => updateBoolSetting('lock_on_disconnect', e.target.checked)}
                />
                <span className="toggle-slider"></span>
              </label>
            </div>
            <div className="settings-item">
              <div className="settings-item-info">
                <span className="settings-item-label">Session timeout</span>
                <span className="settings-item-desc">
                  Automatically disconnect after inactivity
                </span>
              </div>
              <select
                className="settings-select"
                value={settings?.session_timeout ?? 0}
                onChange={(e) => updateNumberSetting('session_timeout', parseInt(e.target.value))}
              >
                <option value="0">Never</option>
                <option value="5">5 minutes</option>
                <option value="15">15 minutes</option>
                <option value="30">30 minutes</option>
                <option value="60">1 hour</option>
              </select>
            </div>
            <div className="settings-info-box info">
              <span className="info-icon">🔐</span>
              <p>
                All connections use <strong>end-to-end encryption</strong> with the Noise Protocol (XK pattern) and ChaCha20-Poly1305. Even our relay servers cannot see your screen data.
              </p>
            </div>
          </div>
        );

      case 'privacy':
        return (
          <div className="settings-category-content">
            <h2>Privacy Settings</h2>
            <div className="settings-item">
              <div className="settings-item-info">
                <span className="settings-item-label">Hide from address book</span>
                <span className="settings-item-desc">
                  Don't allow others to save your device in their address book
                </span>
              </div>
              <label className="toggle-switch">
                <input
                  type="checkbox"
                  checked={settings?.hide_from_address_book ?? false}
                  onChange={(e) => updateBoolSetting('hide_from_address_book', e.target.checked)}
                />
                <span className="toggle-slider"></span>
              </label>
            </div>
            <div className="settings-item">
              <div className="settings-item-info">
                <span className="settings-item-label">Anonymous relay mode</span>
                <span className="settings-item-desc">
                  Force all connections through relay servers (hides your IP)
                </span>
              </div>
              <label className="toggle-switch">
                <input
                  type="checkbox"
                  checked={!(settings?.p2p_enabled ?? true)}
                  onChange={(e) => updateBoolSetting('p2p_enabled', !e.target.checked)}
                />
                <span className="toggle-slider"></span>
              </label>
            </div>
            <div className="settings-info-box success">
              <span className="info-icon">👁️‍🗨️</span>
              <div>
                <p><strong>Zero Telemetry Promise</strong></p>
                <p>SecureDesk does not collect any telemetry, analytics, or usage data. We don't track you, ever.</p>
              </div>
            </div>
          </div>
        );

      case 'license':
        return (
          <div className="settings-category-content">
            <h2>License</h2>

            <div className="license-status-card">
              <div className="license-tier">
                <span className="tier-badge" data-tier={licenseInfo?.tier.toLowerCase() || 'free'}>
                  {licenseInfo?.tier || 'Free'}
                </span>
                {licenseInfo?.days_remaining !== null && licenseInfo?.days_remaining !== undefined && (
                  <span className="license-expiry">
                    {licenseInfo.days_remaining > 0
                      ? `${licenseInfo.days_remaining} days remaining`
                      : licenseInfo.days_remaining === 0
                        ? 'Expires today'
                        : 'Expired'}
                  </span>
                )}
                {licenseInfo?.days_remaining === null && licenseInfo?.tier !== 'Free' && (
                  <span className="license-expiry lifetime">Lifetime license</span>
                )}
              </div>
              {licenseInfo?.key_id && (
                <div className="license-key-id">
                  License ID: {licenseInfo.key_id.substring(0, 8)}...
                </div>
              )}
            </div>

            <div className="license-input-section">
              <label className="settings-label">Enter License Key</label>
              <div className="license-input-row">
                <input
                  type="text"
                  className="license-input"
                  placeholder="XXXX-XXXX-XXXX-XXXX"
                  value={licenseKey}
                  onChange={(e) => setLicenseKey(e.target.value)}
                />
                <button
                  className="license-btn activate"
                  onClick={handleActivateLicense}
                >
                  Activate
                </button>
              </div>
              {licenseError && <div className="license-message error">{licenseError}</div>}
              {licenseSuccess && <div className="license-message success">{licenseSuccess}</div>}
            </div>

            {licenseInfo?.tier !== 'Free' && (
              <div className="license-actions">
                <button
                  className="license-btn deactivate"
                  onClick={handleDeactivateLicense}
                >
                  Deactivate License
                </button>
              </div>
            )}

            <div className="license-features">
              <h3>Features by Tier</h3>
              <div className="features-grid">
                <div className="feature-column">
                  <h4>Free</h4>
                  <ul>
                    <li className="included">Remote control</li>
                    <li className="included">End-to-end encryption</li>
                    <li className="included">P2P connections</li>
                    <li className="excluded">File transfer</li>
                    <li className="excluded">Clipboard sync</li>
                  </ul>
                </div>
                <div className="feature-column">
                  <h4>Basic</h4>
                  <ul>
                    <li className="included">Everything in Free</li>
                    <li className="included">File transfer</li>
                    <li className="included">Clipboard sync</li>
                    <li className="included">Multi-monitor</li>
                    <li className="excluded">Unattended access</li>
                  </ul>
                </div>
                <div className="feature-column highlight">
                  <h4>Pro</h4>
                  <ul>
                    <li className="included">Everything in Basic</li>
                    <li className="included">Unattended access</li>
                    <li className="included">Session recording</li>
                    <li className="included">Custom branding</li>
                    <li className="included">Priority support</li>
                  </ul>
                </div>
              </div>
            </div>

            <div className="license-purchase">
              <p>Don't have a license? Visit our website to purchase.</p>
              <button
                className="purchase-link"
                onClick={() => open('https://securedesk.one/#pricing')}
              >
                View Pricing Plans
              </button>
            </div>
          </div>
        );

      case 'trusted':
        return (
          <div className="settings-category-content">
            <h2>Trusted Devices</h2>
            <p className="settings-description">
              Trusted devices can connect without requiring approval. They will be automatically accepted.
            </p>

            {trustedDevices.length === 0 ? (
              <div className="empty-state">
                <span className="empty-icon">🔓</span>
                <p>No trusted devices yet</p>
                <p className="empty-hint">
                  When accepting a connection, check "Trust this device" to add it here.
                </p>
              </div>
            ) : (
              <div className="trusted-devices-list">
                {trustedDevices.map((device) => (
                  <div key={device.device_id} className="trusted-device-item">
                    <div className="device-info">
                      <span className="device-id">{formatDeviceId(device.device_id)}</span>
                      <span className="device-name">{device.name || 'Unnamed device'}</span>
                      <span className="device-meta">
                        Trusted on {formatDate(device.trusted_at)}
                        {device.last_connected && (
                          <> · Last connected {formatDate(device.last_connected)}</>
                        )}
                      </span>
                    </div>
                    <button
                      className="remove-device-btn"
                      onClick={() => handleRemoveTrustedDevice(device.device_id)}
                      title="Remove from trusted devices"
                    >
                      ×
                    </button>
                  </div>
                ))}
              </div>
            )}
          </div>
        );

      case 'about':
        return (
          <div className="settings-category-content">
            <h2>About SecureDesk</h2>
            <div className="about-logo">
              <div className="logo-icon">🖥️</div>
              <h1>SecureDesk</h1>
              <p className="version">Version 0.1.0</p>
            </div>
            <div className="about-description">
              <p>
                Privacy-preserving remote desktop with end-to-end encryption.
                No telemetry, no tracking, just secure connections.
              </p>
            </div>
            <div className="about-links">
              <button onClick={() => open('https://securedesk.one')}>
                Website
              </button>
              <button onClick={() => open('https://securedesk.one/privacy')}>
                Privacy Policy
              </button>
              <button onClick={() => open('https://securedesk.one/terms')}>
                Terms of Service
              </button>
            </div>
            <div className="about-credits">
              <p>Built with Tauri, React, and Rust</p>
              <p>© 2024 SecureDesk. All rights reserved.</p>
            </div>
          </div>
        );

      default:
        return null;
    }
  };

  return (
    <div className="settings-overlay" onClick={onClose}>
      <div className="settings-modal" onClick={(e) => e.stopPropagation()}>
        <div className="settings-sidebar">
          <div className="settings-sidebar-header">
            <h2>Settings</h2>
          </div>
          <nav className="settings-nav">
            {categories.map((cat) => (
              <button
                key={cat.id}
                className={`settings-nav-item ${activeCategory === cat.id ? 'active' : ''}`}
                onClick={() => setActiveCategory(cat.id)}
              >
                <span className="nav-icon">{cat.icon}</span>
                <span className="nav-label">{cat.label}</span>
              </button>
            ))}
          </nav>
        </div>
        <div className="settings-main">
          <button className="settings-close" onClick={onClose}>×</button>
          {renderContent()}
        </div>
      </div>
    </div>
  );
}

export default Settings;

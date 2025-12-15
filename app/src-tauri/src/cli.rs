//! CLI interface for SecureDesk
//!
//! Provides command-line interface for headless operation and automation.
//! Similar to AnyDesk's command-line interface.
//!
//! Usage:
//!   securedesk                       # Start GUI application
//!   securedesk --id                  # Print device ID and exit
//!   securedesk --new-id              # Generate new device ID
//!   securedesk --get-alias           # Get current alias
//!   securedesk --set-alias NAME      # Set alias
//!   securedesk --version             # Print version
//!   securedesk <address>             # Connect to remote address
//!   securedesk --service             # Start as service/daemon
//!   securedesk --listen              # Start listening for connections (headless)

use clap::{Parser, Subcommand};

/// SecureDesk - Privacy-Preserving Remote Desktop
#[derive(Parser, Debug)]
#[command(name = "securedesk")]
#[command(author = "SecureDesk")]
#[command(version)]
#[command(about = "Privacy-Preserving Remote Desktop Application", long_about = None)]
pub struct Cli {
    /// Print this device's ID and exit
    #[arg(long = "id", short = 'i')]
    pub get_id: bool,

    /// Generate a new device ID
    #[arg(long = "new-id")]
    pub new_id: bool,

    /// Get the current alias
    #[arg(long = "get-alias")]
    pub get_alias: bool,

    /// Set an alias for this device
    #[arg(long = "set-alias", value_name = "NAME")]
    pub set_alias: Option<String>,

    /// Start in service/daemon mode
    #[arg(long = "service")]
    pub service: bool,

    /// Start listening for incoming connections (headless mode)
    #[arg(long = "listen", short = 'l')]
    pub listen: bool,

    /// Set relay server address
    #[arg(long = "relay", value_name = "ADDRESS")]
    pub relay: Option<String>,

    /// Connect to a remote device by ID
    #[arg(value_name = "ADDRESS")]
    pub connect_to: Option<String>,

    /// Run in headless mode (no GUI)
    #[arg(long = "headless")]
    pub headless: bool,

    /// Subcommands for additional operations
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// License management commands
    License {
        #[command(subcommand)]
        action: LicenseAction,
    },
    /// Configuration commands
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Recording commands
    Recording {
        #[command(subcommand)]
        action: RecordingAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum LicenseAction {
    /// Show current license info
    Info,
    /// Activate a license key
    Activate {
        #[arg(value_name = "LICENSE_KEY")]
        key: String,
    },
    /// Deactivate the current license
    Deactivate,
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Show current configuration
    Show,
    /// Get a configuration value
    Get {
        #[arg(value_name = "KEY")]
        key: String,
    },
    /// Set a configuration value
    Set {
        #[arg(value_name = "KEY")]
        key: String,
        #[arg(value_name = "VALUE")]
        value: String,
    },
    /// List trusted devices
    TrustedDevices,
    /// Add a trusted device
    Trust {
        #[arg(value_name = "DEVICE_ID")]
        device_id: String,
        #[arg(long = "name", value_name = "NAME")]
        name: Option<String>,
    },
    /// Remove a trusted device
    Untrust {
        #[arg(value_name = "DEVICE_ID")]
        device_id: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum RecordingAction {
    /// List all recordings
    List,
    /// Show recordings directory
    Dir,
    /// Delete a recording
    Delete {
        #[arg(value_name = "PATH")]
        path: String,
    },
}

impl Cli {
    /// Check if CLI should run in headless/non-GUI mode
    pub fn is_headless_mode(&self) -> bool {
        self.headless
            || self.get_id
            || self.new_id
            || self.get_alias
            || self.set_alias.is_some()
            || self.service
            || self.listen
            || self.command.is_some()
    }
}

/// Handle CLI commands and return true if program should exit
pub fn handle_cli(cli: &Cli) -> Option<i32> {
    use crate::crypto::Identity;
    use crate::config::ConnectionConfig;
    use crate::license::LicenseManager;
    use crate::recording;

    // Handle --id
    if cli.get_id {
        match Identity::load_or_create() {
            Ok(identity) => {
                println!("{}", identity.device_id());
                return Some(0);
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                return Some(1);
            }
        }
    }

    // Handle --new-id
    if cli.new_id {
        match Identity::regenerate() {
            Ok(identity) => {
                println!("New device ID: {}", identity.device_id());
                return Some(0);
            }
            Err(e) => {
                eprintln!("Error generating new ID: {}", e);
                return Some(1);
            }
        }
    }

    // Handle --get-alias
    if cli.get_alias {
        let config = ConnectionConfig::load_or_create().unwrap_or_default();
        match config.get_alias() {
            Some(alias) => {
                println!("{}", alias);
                return Some(0);
            }
            None => {
                println!("(no alias set)");
                return Some(0);
            }
        }
    }

    // Handle --set-alias
    if let Some(ref alias) = cli.set_alias {
        let mut config = ConnectionConfig::load_or_create().unwrap_or_default();
        if let Err(e) = config.set_alias(alias) {
            eprintln!("Error setting alias: {}", e);
            return Some(1);
        }
        println!("Alias set to: {}", alias);
        return Some(0);
    }

    // Handle subcommands
    if let Some(ref command) = cli.command {
        return handle_subcommand(command);
    }

    // No immediate CLI action - continue to GUI or listen mode
    None
}

fn handle_subcommand(command: &Commands) -> Option<i32> {
    use crate::crypto::Identity;
    use crate::config::ConnectionConfig;
    use crate::license::LicenseManager;
    use crate::recording;

    match command {
        Commands::License { action } => {
            let identity = match Identity::load_or_create() {
                Ok(i) => i,
                Err(e) => {
                    eprintln!("Error loading identity: {}", e);
                    return Some(1);
                }
            };
            let mut manager = LicenseManager::new(identity.public_key());
            let _ = manager.load();

            match action {
                LicenseAction::Info => {
                    let info = manager.license_info();
                    println!("License Tier: {}", info.tier);
                    println!("Max Sessions: {}", info.max_sessions);
                    println!("Valid: {}", info.is_valid);
                    if let Some(ref key_id) = info.key_id {
                        println!("Key ID: {}", key_id);
                    }
                    if let Some(exp) = info.expires_at {
                        println!("Expires: {}", exp);
                    }
                    if let Some(days) = info.days_remaining {
                        println!("Days Remaining: {}", days);
                    }
                    Some(0)
                }
                LicenseAction::Activate { key } => {
                    match manager.activate(key) {
                        Ok(tier) => {
                            println!("License activated: {}", tier.as_str());
                            Some(0)
                        }
                        Err(e) => {
                            eprintln!("Activation failed: {}", e);
                            Some(1)
                        }
                    }
                }
                LicenseAction::Deactivate => {
                    match manager.deactivate() {
                        Ok(_) => {
                            println!("License deactivated");
                            Some(0)
                        }
                        Err(e) => {
                            eprintln!("Deactivation failed: {}", e);
                            Some(1)
                        }
                    }
                }
            }
        }
        Commands::Config { action } => {
            let mut config = ConnectionConfig::load_or_create().unwrap_or_default();

            match action {
                ConfigAction::Show => {
                    let settings = config.get_settings();
                    println!("P2P Enabled: {}", settings.p2p_enabled);
                    println!("Require Approval: {}", settings.require_approval);
                    println!("Lock on Disconnect: {}", settings.lock_on_disconnect);
                    println!("Session Timeout: {}s", settings.session_timeout);
                    println!("Start with System: {}", settings.start_with_windows);
                    println!("Minimize to Tray: {}", settings.minimize_to_tray);
                    println!("Show Notifications: {}", settings.show_notifications);
                    println!("Connection Quality: {}", settings.connection_quality);
                    Some(0)
                }
                ConfigAction::Get { key } => {
                    let settings = config.get_settings();
                    let value = match key.as_str() {
                        "p2p_enabled" => format!("{}", settings.p2p_enabled),
                        "require_approval" => format!("{}", settings.require_approval),
                        "lock_on_disconnect" => format!("{}", settings.lock_on_disconnect),
                        "session_timeout" => format!("{}", settings.session_timeout),
                        "start_with_windows" => format!("{}", settings.start_with_windows),
                        "minimize_to_tray" => format!("{}", settings.minimize_to_tray),
                        "show_notifications" => format!("{}", settings.show_notifications),
                        "connection_quality" => settings.connection_quality.clone(),
                        _ => {
                            eprintln!("Unknown config key: {}", key);
                            return Some(1);
                        }
                    };
                    println!("{}", value);
                    Some(0)
                }
                ConfigAction::Set { key, value } => {
                    let setting_value = match key.as_str() {
                        "p2p_enabled" | "require_approval" | "lock_on_disconnect" |
                        "start_with_windows" | "minimize_to_tray" | "show_notifications" => {
                            let bool_val = match value.to_lowercase().as_str() {
                                "true" | "1" | "yes" | "on" => true,
                                "false" | "0" | "no" | "off" => false,
                                _ => {
                                    eprintln!("Invalid boolean value: {}", value);
                                    return Some(1);
                                }
                            };
                            crate::config::SettingValue::Bool(bool_val)
                        }
                        "session_timeout" => {
                            match value.parse::<u32>() {
                                Ok(n) => crate::config::SettingValue::Number(n),
                                Err(_) => {
                                    eprintln!("Invalid number: {}", value);
                                    return Some(1);
                                }
                            }
                        }
                        "connection_quality" => {
                            crate::config::SettingValue::String(value.clone())
                        }
                        _ => {
                            eprintln!("Unknown config key: {}", key);
                            return Some(1);
                        }
                    };

                    if let Err(e) = config.update_setting(key, setting_value) {
                        eprintln!("Error setting {}: {}", key, e);
                        return Some(1);
                    }
                    println!("Set {} = {}", key, value);
                    Some(0)
                }
                ConfigAction::TrustedDevices => {
                    let devices = config.get_trusted_devices();
                    if devices.is_empty() {
                        println!("No trusted devices");
                    } else {
                        for device in devices {
                            let name = device.name.as_deref().unwrap_or("(unnamed)");
                            println!("{} - {}", device.device_id, name);
                        }
                    }
                    Some(0)
                }
                ConfigAction::Trust { device_id, name } => {
                    if let Err(e) = config.add_trusted_device(device_id, name.clone()) {
                        eprintln!("Error adding trusted device: {}", e);
                        return Some(1);
                    }
                    println!("Device {} trusted", device_id);
                    Some(0)
                }
                ConfigAction::Untrust { device_id } => {
                    if let Err(e) = config.remove_trusted_device(device_id) {
                        eprintln!("Error removing trusted device: {}", e);
                        return Some(1);
                    }
                    println!("Device {} removed from trusted list", device_id);
                    Some(0)
                }
            }
        }
        Commands::Recording { action } => {
            match action {
                RecordingAction::List => {
                    match recording::list_recordings() {
                        Ok(recordings) => {
                            if recordings.is_empty() {
                                println!("No recordings found");
                            } else {
                                for rec in recordings {
                                    let duration_secs = rec.duration_ms / 1000;
                                    let mins = duration_secs / 60;
                                    let secs = duration_secs % 60;
                                    println!("{} - {}:{:02} - {} - {} frames",
                                        rec.filename,
                                        mins, secs,
                                        rec.remote_device_name,
                                        rec.frame_count
                                    );
                                }
                            }
                            Some(0)
                        }
                        Err(e) => {
                            eprintln!("Error listing recordings: {}", e);
                            Some(1)
                        }
                    }
                }
                RecordingAction::Dir => {
                    match recording::SessionRecorder::recordings_directory() {
                        Ok(dir) => {
                            println!("{}", dir.display());
                            Some(0)
                        }
                        Err(e) => {
                            eprintln!("Error: {}", e);
                            Some(1)
                        }
                    }
                }
                RecordingAction::Delete { path } => {
                    match recording::delete_recording(path) {
                        Ok(_) => {
                            println!("Recording deleted");
                            Some(0)
                        }
                        Err(e) => {
                            eprintln!("Error deleting recording: {}", e);
                            Some(1)
                        }
                    }
                }
            }
        }
    }
}

/// Run headless listen mode
pub async fn run_headless_listen(relay_address: Option<String>) -> anyhow::Result<()> {
    use crate::crypto::Identity;
    use crate::host::HostSession;

    let identity = Identity::load_or_create()?;
    println!("Device ID: {}", identity.device_id());

    let relay = relay_address.unwrap_or_else(|| "relay.securedesk.one:8443".to_string());
    println!("Connecting to relay: {}", relay);

    let mut session = HostSession::start(relay, identity).await?;
    println!("Listening for incoming connections...");
    println!("Press Ctrl+C to stop");

    // Run the host session loop
    loop {
        if let Err(e) = session.run_once().await {
            eprintln!("Host session error: {}", e);
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }
}

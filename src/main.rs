use std::error::Error;
use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::prelude::{Backend, CrosstermBackend, Terminal};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Axis, Block, Borders, Chart, Dataset, GraphType, List, ListItem, Paragraph, Row, Table, Tabs,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use chrono::Utc;
use solana_sdk::signature::{Keypair, Signer};

const KEYCHAIN_SERVICE: &str = "den-wallet";
const KEYCHAIN_API_KEY_ACCOUNT: &str = "helius-api-key";
const CONFIG_DIR_NAME: &str = "den";
const CONFIG_FILE_NAME: &str = "config.toml";
const CONFIG_CACHE_FILE_NAME: &str = "config-cache.json";
const BOOTSTRAP_FILE_NAME: &str = "bootstrap.json";
const CONTACTS_FILE_NAME: &str = "contacts.json";
const CONFIG_BACKEND_ENV: &str = "DEN_CONFIG_BACKEND";
const BW_CONFIG_ITEM_ID_ENV: &str = "DEN_BW_CONFIG_ITEM_ID";

static CONFIG_REV: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static BW_SESSION_CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();

const COLOR_BARK: Color = Color::Rgb(58, 46, 42);
const COLOR_FAWN: Color = Color::Rgb(199, 181, 154);
const COLOR_ASH: Color = Color::Rgb(232, 225, 215);
const COLOR_SOOT: Color = Color::Rgb(16, 16, 16);
const COLOR_STONE: Color = Color::Rgb(118, 111, 102);
const COLOR_MOSS: Color = Color::Rgb(78, 104, 82);
const COLOR_EMBER: Color = Color::Rgb(179, 106, 78);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Overview,
    Accounts,
    Tokens,
    Send,
    Receive,
    History,
    AddressBook,
    Settings,
}

impl Tab {
    const ALL: [Tab; 8] = [
        Tab::Overview,
        Tab::Accounts,
        Tab::Tokens,
        Tab::Send,
        Tab::Receive,
        Tab::History,
        Tab::AddressBook,
        Tab::Settings,
    ];

    fn title(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Accounts => "Accounts",
            Tab::Tokens => "Tokens",
            Tab::Send => "Send",
            Tab::Receive => "Receive",
            Tab::History => "History",
            Tab::AddressBook => "Address Book",
            Tab::Settings => "Settings",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }
}

struct Token {
    symbol: String,
    balance: String,
    value: String,
    history: Vec<f64>,
}

struct Account {
    id: String,
    name: String,
    address: String,
    balance: String,
    has_key: bool,
    is_active: bool,
    added_at: Option<String>,
}

struct Transaction {
    time: String,
    summary: String,
    amount: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Contact {
    name: String,
    address: String,
    #[serde(default = "default_contact_network")]
    network: String,
    #[serde(default)]
    notes: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct ContactsFile {
    #[serde(default = "default_contacts_version")]
    version: u32,
    #[serde(default)]
    contacts: Vec<Contact>,
}

fn default_contact_network() -> String {
    "mainnet".to_string()
}

fn default_contacts_version() -> u32 {
    1
}

struct WalletData {
    sol_balance: f64,
    tokens: Vec<Token>,
    history: Vec<Transaction>,
}

struct Config {
    address: String,
    rpc_url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Network {
    Mainnet,
    Devnet,
}

impl Network {
    fn toggle(self) -> Self {
        match self {
            Network::Mainnet => Network::Devnet,
            Network::Devnet => Network::Mainnet,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Network::Mainnet => "Mainnet",
            Network::Devnet => "Devnet",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct DenConfig {
    #[serde(default)]
    network: NetworkConfig,
    #[serde(default)]
    display: DisplayConfig,
    #[serde(default)]
    active_wallet: Option<String>,
    #[serde(default)]
    wallets: Vec<WalletEntry>,
    #[serde(default, skip_serializing)]
    wallet: Option<LegacyWalletConfig>,
}

impl Default for DenConfig {
    fn default() -> Self {
        Self {
            network: NetworkConfig::default(),
            display: DisplayConfig::default(),
            active_wallet: None,
            wallets: Vec::new(),
            wallet: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct WalletEntry {
    id: String,
    name: String,
    address: String,
    #[serde(default)]
    has_key: bool,
    #[serde(default)]
    added_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct LegacyWalletConfig {
    #[serde(default)]
    address: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct NetworkConfig {
    #[serde(default = "default_network")]
    default: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    api_key: Option<String>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            default: default_network(),
            api_key: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct DisplayConfig {
    #[serde(default = "default_theme")]
    theme: String,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
        }
    }
}

fn default_network() -> String {
    "mainnet".to_string()
}

fn default_theme() -> String {
    "den".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ConfigEnvelope {
    config: DenConfig,
    rev: String,
    updated_at: String,
    updated_by: String,
}

impl ConfigEnvelope {
    fn from_config(config: DenConfig) -> Self {
        Self {
            config,
            rev: new_config_rev(),
            updated_at: Utc::now().to_rfc3339(),
            updated_by: std::env::var("HOSTNAME")
                .or_else(|_| std::env::var("USER"))
                .unwrap_or_else(|_| "unknown".to_string()),
        }
    }
}

trait ConfigStore {
    fn load(&self) -> Result<ConfigEnvelope, Box<dyn Error>>;
    fn save(
        &self,
        config: &DenConfig,
        expected_rev: Option<&str>,
    ) -> Result<ConfigEnvelope, Box<dyn Error>>;
    fn location(&self) -> String;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfigBackend {
    Local,
    Bitwarden,
}

struct LocalConfigStore;

struct BitwardenConfigStore {
    item_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct BootstrapConfig {
    #[serde(default)]
    backend: Option<String>,
    #[serde(default)]
    bitwarden_item_id: Option<String>,
    #[serde(default)]
    onboarding_complete: bool,
}

fn config_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|dir| dir.join(CONFIG_DIR_NAME).join(CONFIG_FILE_NAME))
}

fn bootstrap_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|dir| dir.join(CONFIG_DIR_NAME).join(BOOTSTRAP_FILE_NAME))
}

fn config_cache_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|dir| dir.join(CONFIG_DIR_NAME).join(CONFIG_CACHE_FILE_NAME))
}

fn load_bootstrap_config() -> BootstrapConfig {
    let path = match bootstrap_path() {
        Some(path) => path,
        None => return BootstrapConfig::default(),
    };

    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => BootstrapConfig::default(),
    }
}

fn save_bootstrap_config(config: &BootstrapConfig) -> Result<(), Box<dyn Error>> {
    let path = bootstrap_path().ok_or("Cannot determine config directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let contents = serde_json::to_string_pretty(config)?;
    std::fs::write(path, contents)?;
    Ok(())
}

fn should_start_onboarding() -> bool {
    if std::env::var(CONFIG_BACKEND_ENV).is_ok() || std::env::var(BW_CONFIG_ITEM_ID_ENV).is_ok() {
        return false;
    }
    let bootstrap = load_bootstrap_config();
    !bootstrap.onboarding_complete
}

fn current_config_backend() -> ConfigBackend {
    if let Ok(value) = std::env::var(CONFIG_BACKEND_ENV) {
        return match value.to_ascii_lowercase().as_str() {
            "bitwarden" | "bw" => ConfigBackend::Bitwarden,
            _ => ConfigBackend::Local,
        };
    }

    let bootstrap = load_bootstrap_config();
    if let Some(value) = bootstrap.backend {
        return match value.to_ascii_lowercase().as_str() {
            "bitwarden" | "bw" => ConfigBackend::Bitwarden,
            _ => ConfigBackend::Local,
        };
    }

    ConfigBackend::Local
}

fn resolve_bitwarden_item_id() -> Option<String> {
    if let Ok(item_id) = std::env::var(BW_CONFIG_ITEM_ID_ENV) {
        if !item_id.trim().is_empty() {
            return Some(item_id);
        }
    }

    let bootstrap = load_bootstrap_config();
    bootstrap.bitwarden_item_id.filter(|id| !id.trim().is_empty())
}

fn selected_config_store() -> Result<Box<dyn ConfigStore>, Box<dyn Error>> {
    match current_config_backend() {
        ConfigBackend::Local => Ok(Box::new(LocalConfigStore)),
        ConfigBackend::Bitwarden => {
            let item_id = resolve_bitwarden_item_id()
                .ok_or(format!("{} is not set", BW_CONFIG_ITEM_ID_ENV))?;
            Ok(Box::new(BitwardenConfigStore { item_id }))
        }
    }
}

fn config_rev_cell() -> &'static Mutex<Option<String>> {
    CONFIG_REV.get_or_init(|| Mutex::new(None))
}

fn bw_session_cell() -> &'static Mutex<Option<String>> {
    BW_SESSION_CACHE.get_or_init(|| Mutex::new(None))
}

fn set_cached_bw_session(session: Option<String>) {
    if let Ok(mut guard) = bw_session_cell().lock() {
        *guard = session;
    }
}

fn cached_bw_session() -> Option<String> {
    bw_session_cell().lock().ok().and_then(|g| g.clone())
}

fn set_cached_config_rev(rev: Option<String>) {
    if let Ok(mut guard) = config_rev_cell().lock() {
        *guard = rev;
    }
}

fn cached_config_rev() -> Option<String> {
    config_rev_cell().lock().ok().and_then(|g| g.clone())
}

fn new_config_rev() -> String {
    let nanos = Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or(Utc::now().timestamp_micros() * 1_000);
    format!("rev-{}-{}", nanos, std::process::id())
}

fn load_cached_config_envelope() -> Option<ConfigEnvelope> {
    let path = config_cache_path()?;
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

fn save_cached_config_envelope(envelope: &ConfigEnvelope) {
    let Some(path) = config_cache_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(contents) = serde_json::to_string_pretty(envelope) {
        let _ = std::fs::write(path, contents);
    }
}

fn parse_config_envelope(raw: &str) -> Result<ConfigEnvelope, Box<dyn Error>> {
    if raw.trim().is_empty() {
        return Err("Empty config payload".into());
    }

    if let Ok(envelope) = serde_json::from_str::<ConfigEnvelope>(raw) {
        return Ok(envelope);
    }

    if let Ok(config) = toml::from_str::<DenConfig>(raw) {
        return Ok(ConfigEnvelope::from_config(config));
    }

    let config = serde_json::from_str::<DenConfig>(raw)?;
    Ok(ConfigEnvelope::from_config(config))
}

fn run_command_with_input_and_env(
    cmd: &str,
    args: &[&str],
    input: Option<&str>,
    env_vars: &[(&str, &str)],
) -> Result<String, Box<dyn Error>> {
    let mut command = Command::new(cmd);
    command.args(args);
    if cmd == "bw" {
        if let Some(session) = cached_bw_session() {
            if !session.trim().is_empty() {
                command.env("BW_SESSION", session);
            }
        }
    }
    for (key, value) in env_vars {
        command.env(key, value);
    }
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;

    if let Some(payload) = input {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(payload.as_bytes())?;
        }
    }

    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        let message = if detail.is_empty() {
            format!("Command '{}' failed", cmd)
        } else {
            format!("Command '{}' failed: {}", cmd, detail)
        };
        return Err(message.into());
    }

    Ok(String::from_utf8(output.stdout)?)
}

fn run_command_with_input(
    cmd: &str,
    args: &[&str],
    input: Option<&str>,
) -> Result<String, Box<dyn Error>> {
    run_command_with_input_and_env(cmd, args, input, &[])
}

fn bw_encode(payload: &str) -> Result<String, Box<dyn Error>> {
    let encoded = run_command_with_input("bw", &["encode"], Some(payload))?;
    Ok(encoded.trim().to_string())
}

fn bw_get_item_json(item_id: &str) -> Result<serde_json::Value, Box<dyn Error>> {
    let output = run_command_with_input("bw", &["get", "item", item_id], None)?;
    Ok(serde_json::from_str(&output)?)
}

fn bw_edit_item_partial(item_id: &str, payload: &serde_json::Value) -> Result<(), Box<dyn Error>> {
    let payload_json = serde_json::to_string(payload)?;
    let encoded = bw_encode(&payload_json)?;
    let _ = run_command_with_input("bw", &["edit", "item", item_id, &encoded], None)?;
    Ok(())
}

fn bw_status() -> Result<String, Box<dyn Error>> {
    let output = run_command_with_input("bw", &["status", "--raw"], None)?;
    let parsed: serde_json::Value = serde_json::from_str(&output)?;
    let status = parsed
        .get("status")
        .and_then(|v| v.as_str())
        .ok_or("Unable to determine Bitwarden status")?;
    Ok(status.to_string())
}

fn bw_login_with_apikey(client_id: &str, client_secret: &str) -> Result<(), Box<dyn Error>> {
    let _ = run_command_with_input_and_env(
        "bw",
        &["login", "--apikey"],
        None,
        &[("BW_CLIENTID", client_id), ("BW_CLIENTSECRET", client_secret)],
    )?;
    Ok(())
}

fn bw_unlock_with_password(password: &str) -> Result<String, Box<dyn Error>> {
    let session = run_command_with_input_and_env(
        "bw",
        &["unlock", "--raw", "--passwordenv", "BW_PASSWORD"],
        None,
        &[("BW_PASSWORD", password)],
    )?;
    let token = session.trim().to_string();
    if token.is_empty() {
        return Err("Bitwarden unlock did not return a session token".into());
    }
    Ok(token)
}

impl ConfigStore for LocalConfigStore {
    fn load(&self) -> Result<ConfigEnvelope, Box<dyn Error>> {
        let path = config_path().ok_or("Cannot determine config directory")?;
        let config: DenConfig = match std::fs::read_to_string(path) {
            Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
            Err(_) => DenConfig::default(),
        };
        Ok(ConfigEnvelope::from_config(config))
    }

    fn save(
        &self,
        config: &DenConfig,
        _expected_rev: Option<&str>,
    ) -> Result<ConfigEnvelope, Box<dyn Error>> {
        let path = config_path().ok_or("Cannot determine config directory")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(config)?;
        std::fs::write(&path, contents)?;
        Ok(ConfigEnvelope::from_config(config.clone()))
    }

    fn location(&self) -> String {
        config_path()
            .map(|p| format!("{}", p.display()))
            .unwrap_or_else(|| "unavailable".to_string())
    }
}

impl ConfigStore for BitwardenConfigStore {
    fn load(&self) -> Result<ConfigEnvelope, Box<dyn Error>> {
        let item = bw_get_item_json(&self.item_id)?;
        let notes = item
            .get("notes")
            .and_then(|n| n.as_str())
            .ok_or("Bitwarden config item is missing notes")?;
        parse_config_envelope(notes)
    }

    fn save(
        &self,
        config: &DenConfig,
        expected_rev: Option<&str>,
    ) -> Result<ConfigEnvelope, Box<dyn Error>> {
        let current = self.load().ok();
        if let (Some(expected), Some(existing)) = (expected_rev, current.as_ref()) {
            if existing.rev != expected {
                return Err(format!(
                    "Config conflict: expected rev {}, found {}",
                    expected, existing.rev
                )
                .into());
            }
        }

        let envelope = ConfigEnvelope::from_config(config.clone());
        let notes = serde_json::to_string_pretty(&envelope)?;
        let payload = json!({ "notes": notes });
        bw_edit_item_partial(&self.item_id, &payload)?;
        Ok(envelope)
    }

    fn location(&self) -> String {
        format!("bitwarden:{}", self.item_id)
    }
}

fn config_location_display() -> String {
    selected_config_store()
        .map(|store| store.location())
        .unwrap_or_else(|_| "unavailable".to_string())
}

fn persist_backend_choice(backend: ConfigBackend, bitwarden_item_id: Option<String>) -> Result<(), Box<dyn Error>> {
    let mut bootstrap = load_bootstrap_config();
    bootstrap.backend = Some(match backend {
        ConfigBackend::Local => "local".to_string(),
        ConfigBackend::Bitwarden => "bitwarden".to_string(),
    });
    bootstrap.bitwarden_item_id = bitwarden_item_id;
    bootstrap.onboarding_complete = true;
    save_bootstrap_config(&bootstrap)
}

fn initialize_bitwarden_config_item(item_id: &str) -> Result<(), Box<dyn Error>> {
    let item = bw_get_item_json(item_id)?;
    let notes = item
        .get("notes")
        .and_then(|n| n.as_str())
        .unwrap_or("");

    if parse_config_envelope(notes).is_ok() {
        return Ok(());
    }

    let envelope = ConfigEnvelope::from_config(DenConfig::default());
    let payload = json!({
        "notes": serde_json::to_string_pretty(&envelope)?
    });
    bw_edit_item_partial(item_id, &payload)?;
    Ok(())
}

fn migrate_local_config_to_bitwarden(force: bool) -> Result<String, Box<dyn Error>> {
    let item_id = std::env::var(BW_CONFIG_ITEM_ID_ENV)
        .map_err(|_| format!("{} is not set", BW_CONFIG_ITEM_ID_ENV))?;
    let local_store = LocalConfigStore;
    let bitwarden_store = BitwardenConfigStore { item_id };

    if !force && bitwarden_store.load().is_ok() {
        return Err(
            "Bitwarden config already exists. Re-run with --migrate-config-to-bitwarden --force"
                .into(),
        );
    }

    let local = local_store.load()?.config;
    let saved = bitwarden_store.save(&local, None)?;
    persist_backend_choice(
        ConfigBackend::Bitwarden,
        Some(bitwarden_store.item_id.clone()),
    )?;
    set_cached_config_rev(Some(saved.rev.clone()));
    save_cached_config_envelope(&saved);
    Ok(bitwarden_store.location())
}

fn load_den_config() -> DenConfig {
    let store = match selected_config_store() {
        Ok(store) => store,
        Err(_) => return DenConfig::default(),
    };

    let mut envelope = match store.load() {
        Ok(envelope) => {
            save_cached_config_envelope(&envelope);
            envelope
        }
        Err(_) => match load_cached_config_envelope() {
            Some(cached) => cached,
            None => ConfigEnvelope::from_config(DenConfig::default()),
        },
    };

    set_cached_config_rev(Some(envelope.rev.clone()));

    if migrate_config_if_needed(&mut envelope.config) {
        let _ = save_den_config(&envelope.config);
    }

    envelope.config
}

fn migrate_config_if_needed(config: &mut DenConfig) -> bool {
    let legacy = match config.wallet.take() {
        Some(legacy) => legacy,
        None => return false,
    };

    if !config.wallets.is_empty() {
        return false;
    }

    let has_key = keyring::Entry::new(KEYCHAIN_SERVICE, "main")
        .and_then(|e| e.get_password())
        .is_ok();

    let address = if !legacy.address.is_empty() {
        legacy.address
    } else if has_key {
        load_secret_for_wallet("main")
            .ok()
            .and_then(|s| keypair_from_secret(&s).ok())
            .map(|kp| kp.pubkey().to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };

    if address.is_empty() {
        return false;
    }

    let wallet_id = "wallet-0".to_string();

    if has_key {
        if let Ok(secret) = load_secret_for_wallet("main") {
            let _ = store_secret_for_wallet(&wallet_id, &secret);
        }
    }

    config.wallets.push(WalletEntry {
        id: wallet_id.clone(),
        name: "Main".to_string(),
        address,
        has_key,
        added_at: None,
    });
    config.active_wallet = Some(wallet_id);

    true
}

fn next_wallet_id(config: &DenConfig) -> String {
    let max = config
        .wallets
        .iter()
        .filter_map(|w| w.id.strip_prefix("wallet-"))
        .filter_map(|s| s.parse::<u32>().ok())
        .max();
    match max {
        Some(n) => format!("wallet-{}", n + 1),
        None if config.wallets.is_empty() => "wallet-0".to_string(),
        None => format!("wallet-{}", config.wallets.len()),
    }
}

fn active_wallet(config: &DenConfig) -> Option<&WalletEntry> {
    let active_id = config.active_wallet.as_deref()?;
    config.wallets.iter().find(|w| w.id == active_id)
}

fn set_active_wallet(config: &mut DenConfig, wallet_id: &str) {
    if config.wallets.iter().any(|w| w.id == wallet_id) {
        config.active_wallet = Some(wallet_id.to_string());
    }
}

fn save_den_config(config: &DenConfig) -> Result<(), Box<dyn Error>> {
    let store = selected_config_store()?;
    let expected = cached_config_rev();
    let envelope = store.save(config, expected.as_deref())?;
    set_cached_config_rev(Some(envelope.rev.clone()));
    save_cached_config_envelope(&envelope);
    Ok(())
}

fn ensure_config_exists() {
    if current_config_backend() == ConfigBackend::Local {
        if let Some(path) = config_path() {
            if !path.exists() {
                let _ = save_den_config(&DenConfig::default());
            }
        }
    }
}

fn contacts_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|dir| dir.join(CONFIG_DIR_NAME).join(CONTACTS_FILE_NAME))
}

fn load_contacts() -> ContactsFile {
    let path = match contacts_path() {
        Some(path) => path,
        None => return ContactsFile::default(),
    };

    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => ContactsFile::default(),
    }
}

fn save_contacts(file: &ContactsFile) -> Result<(), Box<dyn Error>> {
    let path = contacts_path().ok_or("Cannot determine config directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let contents = serde_json::to_string_pretty(file)?;
    std::fs::write(&path, contents)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputMode {
    None,
    ImportKeyName,
    ImportKey,
    AddWatchOnlyName,
    AddWatchOnly,
    RenameWallet,
    ConfirmDeleteWallet,
    SignMessage,
    AddContactName,
    AddContactAddress,
    EditContactName,
    EditContactAddress,
    EditContactNotes,
    ConfirmDeleteContact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OnboardingStep {
    ChooseBackend,
    BitwardenAuth,
    BitwardenApiKeyId,
    BitwardenApiKeySecret,
    BitwardenMasterPassword,
    BitwardenItemId,
}

struct OnboardingState {
    active: bool,
    step: OnboardingStep,
    input: String,
    message: String,
    bw_client_id: String,
}

struct ImportState {
    wallet_name: String,
}

struct App {
    should_quit: bool,
    tab: Tab,
    accounts: Vec<Account>,
    tokens: Vec<Token>,
    history: Vec<Transaction>,
    contacts: Vec<Contact>,
    selected_account: usize,
    selected_token: usize,
    selected_history: usize,
    selected_contact: usize,
    total_balance: String,
    wallet_address: String,
    active_wallet_id: Option<String>,
    status: String,
    keystore_status: String,
    api_key_status: String,
    default_network: String,
    config_path_display: String,
    network: Network,
    input_mode: InputMode,
    input_buffer: String,
    import_state: ImportState,
    wallet_detail_index: Option<usize>,
    contact_detail_index: Option<usize>,
    last_signature: String,
    onboarding: OnboardingState,
}

impl App {
    fn new_placeholder() -> Self {
        Self {
            should_quit: false,
            tab: Tab::Overview,
            accounts: Vec::new(),
            tokens: vec![Token {
                symbol: "SOL".to_string(),
                balance: "0.00".to_string(),
                value: "-".to_string(),
                history: seeded_series("SOL", 16),
            }],
            history: vec![Transaction {
                time: "".to_string(),
                summary: "No transactions".to_string(),
                amount: "".to_string(),
            }],
            contacts: Vec::new(),
            selected_account: 0,
            selected_token: 0,
            selected_history: 0,
            selected_contact: 0,
            total_balance: "0.00 SOL".to_string(),
            wallet_address: "Unset".to_string(),
            active_wallet_id: None,
            status: "Add a wallet: press 'a' on Accounts tab or run: den --add-wallet <name>".to_string(),
            keystore_status: "Keychain: no wallets".to_string(),
            api_key_status: "API Key: not set".to_string(),
            default_network: "mainnet".to_string(),
            config_path_display: config_location_display(),
            network: Network::Mainnet,
            input_mode: InputMode::None,
            input_buffer: String::new(),
            import_state: ImportState {
                wallet_name: String::new(),
            },
            wallet_detail_index: None,
            contact_detail_index: None,
            last_signature: "-".to_string(),
            onboarding: OnboardingState {
                active: false,
                step: OnboardingStep::ChooseBackend,
                input: String::new(),
                message: String::new(),
                bw_client_id: String::new(),
            },
        }
    }

    fn apply_active_data(&mut self, data: WalletData) {
        self.total_balance = format!("{:.4} SOL", data.sol_balance);
        self.tokens = data.tokens;
        self.history = if data.history.is_empty() {
            vec![Transaction {
                time: "".to_string(),
                summary: "No transactions".to_string(),
                amount: "".to_string(),
            }]
        } else {
            data.history
        };
        self.status = "Live data from Helius".to_string();
    }

    fn on_key(&mut self, code: KeyCode) {
        if self.onboarding.active {
            self.handle_onboarding_mode(code);
            return;
        }

        if self.input_mode != InputMode::None {
            self.handle_input_mode(code);
            return;
        }

        match code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('1') => self.tab = Tab::Overview,
            KeyCode::Char('2') => self.tab = Tab::Accounts,
            KeyCode::Char('3') => self.tab = Tab::Tokens,
            KeyCode::Char('4') => self.tab = Tab::Send,
            KeyCode::Char('5') => self.tab = Tab::Receive,
            KeyCode::Char('6') => self.tab = Tab::History,
            KeyCode::Char('7') => self.tab = Tab::AddressBook,
            KeyCode::Char('8') => self.tab = Tab::Settings,
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::Char('n') => {
                self.network = self.network.toggle();
                let msg = format!("Network set to {}", self.network.label());
                refresh_wallet_data(self);
                self.status = msg;
            }
            KeyCode::Char('r') => {
                refresh_wallet_data(self);
            }
            KeyCode::Char('i') => {
                self.input_mode = InputMode::ImportKeyName;
                self.input_buffer.clear();
                self.import_state.wallet_name.clear();
            }
            KeyCode::Char('a') => {
                if self.tab == Tab::AddressBook {
                    if self.contact_detail_index.is_some() {
                        if let Some(idx) = self.contact_detail_index {
                            if idx < self.contacts.len() {
                                self.input_mode = InputMode::EditContactAddress;
                                self.input_buffer = self.contacts[idx].address.clone();
                            }
                        }
                    } else {
                        self.input_mode = InputMode::AddContactName;
                        self.input_buffer.clear();
                        self.import_state.wallet_name.clear();
                    }
                } else {
                    self.input_mode = InputMode::ImportKeyName;
                    self.input_buffer.clear();
                    self.import_state.wallet_name.clear();
                }
            }
            KeyCode::Char('w') => {
                if self.tab == Tab::Accounts {
                    self.input_mode = InputMode::AddWatchOnlyName;
                    self.input_buffer.clear();
                    self.import_state.wallet_name.clear();
                }
            }
            KeyCode::Char('e') => {
                if self.tab == Tab::Accounts && !self.accounts.is_empty() {
                    self.input_mode = InputMode::RenameWallet;
                    self.input_buffer = self.accounts[self.selected_account].name.clone();
                } else if self.tab == Tab::AddressBook && !self.contacts.is_empty() {
                    let idx = self.contact_detail_index.unwrap_or(self.selected_contact);
                    if idx < self.contacts.len() {
                        self.input_mode = InputMode::EditContactName;
                        self.input_buffer = self.contacts[idx].name.clone();
                    }
                }
            }
            KeyCode::Char('d') => {
                if self.tab == Tab::Accounts && !self.accounts.is_empty() {
                    self.input_mode = InputMode::ConfirmDeleteWallet;
                    self.input_buffer.clear();
                } else if self.tab == Tab::AddressBook && !self.contacts.is_empty() {
                    self.input_mode = InputMode::ConfirmDeleteContact;
                    self.input_buffer.clear();
                }
            }
            KeyCode::Enter => {
                if self.tab == Tab::Accounts && !self.accounts.is_empty() {
                    if self.wallet_detail_index.is_some() {
                        let selected = &self.accounts[self.selected_account];
                        let wallet_id = selected.id.clone();
                        let wallet_name = selected.name.clone();
                        let mut config = load_den_config();
                        set_active_wallet(&mut config, &wallet_id);
                        let _ = save_den_config(&config);
                        let msg = format!("Switched to '{}'", wallet_name);
                        refresh_wallet_data(self);
                        self.status = msg;
                    } else {
                        self.wallet_detail_index = Some(self.selected_account);
                    }
                } else if self.tab == Tab::AddressBook
                    && self.contact_detail_index.is_none()
                    && !self.contacts.is_empty()
                {
                    self.contact_detail_index = Some(self.selected_contact);
                }
            }
            KeyCode::Esc => {
                if self.tab == Tab::Accounts && self.wallet_detail_index.is_some() {
                    self.wallet_detail_index = None;
                } else if self.tab == Tab::AddressBook && self.contact_detail_index.is_some() {
                    self.contact_detail_index = None;
                }
            }
            KeyCode::Char('o') => {
                if self.tab == Tab::AddressBook {
                    if let Some(idx) = self.contact_detail_index {
                        if idx < self.contacts.len() {
                            self.input_mode = InputMode::EditContactNotes;
                            self.input_buffer = self.contacts[idx].notes.clone();
                        }
                    }
                } else if self.tab == Tab::Settings {
                    self.start_onboarding();
                }
            }
            KeyCode::Char('s') => {
                let config = load_den_config();
                match active_wallet(&config) {
                    Some(w) if w.has_key => {
                        self.input_mode = InputMode::SignMessage;
                        self.input_buffer.clear();
                    }
                    Some(w) => {
                        self.status = format!("Cannot sign: '{}' is watch-only", w.name);
                    }
                    None => {
                        self.status = "No active wallet".to_string();
                    }
                }
            }
            _ => {}
        }
    }

    fn start_onboarding(&mut self) {
        self.onboarding.active = true;
        self.onboarding.step = OnboardingStep::ChooseBackend;
        self.onboarding.input.clear();
        self.onboarding.bw_client_id.clear();
        self.onboarding.message = "Choose where config should live.".to_string();
        self.tab = Tab::Settings;
    }

    fn complete_onboarding(&mut self, status: &str) {
        self.onboarding.active = false;
        self.onboarding.step = OnboardingStep::ChooseBackend;
        self.onboarding.input.clear();
        self.onboarding.bw_client_id.clear();
        self.onboarding.message.clear();
        self.config_path_display = config_location_display();
        refresh_wallet_data(self);
        self.status = status.to_string();
    }

    fn handle_onboarding_mode(&mut self, code: KeyCode) {
        match self.onboarding.step {
            OnboardingStep::ChooseBackend => match code {
                KeyCode::Char('1') => {
                    match persist_backend_choice(ConfigBackend::Local, None) {
                        Ok(_) => self.complete_onboarding("Setup complete: using local config"),
                        Err(err) => {
                            self.onboarding.message = format!("Setup failed: {}", err);
                        }
                    }
                }
                KeyCode::Char('2') => {
                    self.onboarding.step = OnboardingStep::BitwardenAuth;
                    self.onboarding.input.clear();
                    self.onboarding.message = match bw_status() {
                        Ok(status) => format!(
                            "Bitwarden status: {}. Press c=check, k=API login, u=unlock, i=continue.",
                            status
                        ),
                        Err(err) => format!("Bitwarden check failed: {}. Press c to retry.", err),
                    };
                }
                KeyCode::Char('q') => self.should_quit = true,
                _ => {}
            },
            OnboardingStep::BitwardenAuth => match code {
                KeyCode::Esc => {
                    self.onboarding.step = OnboardingStep::ChooseBackend;
                    self.onboarding.message = "Choose where config should live.".to_string();
                }
                KeyCode::Char('c') => {
                    self.onboarding.message = match bw_status() {
                        Ok(status) => format!(
                            "Bitwarden status: {}. Press c=check, k=API login, u=unlock, i=continue.",
                            status
                        ),
                        Err(err) => format!("Bitwarden check failed: {}", err),
                    };
                }
                KeyCode::Char('k') => {
                    self.onboarding.step = OnboardingStep::BitwardenApiKeyId;
                    self.onboarding.input.clear();
                    self.onboarding.message = "Enter Bitwarden API client ID.".to_string();
                }
                KeyCode::Char('u') => {
                    self.onboarding.step = OnboardingStep::BitwardenMasterPassword;
                    self.onboarding.input.clear();
                    self.onboarding.message = "Enter Bitwarden master password.".to_string();
                }
                KeyCode::Char('i') => {
                    match bw_status() {
                        Ok(status) if status == "unlocked" => {
                            self.onboarding.step = OnboardingStep::BitwardenItemId;
                            self.onboarding.input.clear();
                            self.onboarding.message =
                                "Enter Bitwarden item ID (secure note).".to_string();
                        }
                        Ok(status) => {
                            self.onboarding.message =
                                format!("Bitwarden is '{}'. Login/unlock first.", status);
                        }
                        Err(err) => {
                            self.onboarding.message = format!("Bitwarden check failed: {}", err);
                        }
                    }
                }
                _ => {}
            },
            OnboardingStep::BitwardenApiKeyId => match code {
                KeyCode::Esc => {
                    self.onboarding.step = OnboardingStep::BitwardenAuth;
                    self.onboarding.input.clear();
                }
                KeyCode::Backspace => {
                    self.onboarding.input.pop();
                }
                KeyCode::Char(ch) => {
                    self.onboarding.input.push(ch);
                }
                KeyCode::Enter => {
                    let client_id = self.onboarding.input.trim().to_string();
                    if client_id.is_empty() {
                        self.onboarding.message = "Client ID cannot be empty.".to_string();
                    } else {
                        self.onboarding.bw_client_id = client_id;
                        self.onboarding.input.clear();
                        self.onboarding.step = OnboardingStep::BitwardenApiKeySecret;
                        self.onboarding.message = "Enter Bitwarden API client secret.".to_string();
                    }
                }
                _ => {}
            },
            OnboardingStep::BitwardenApiKeySecret => match code {
                KeyCode::Esc => {
                    self.onboarding.step = OnboardingStep::BitwardenAuth;
                    self.onboarding.input.clear();
                }
                KeyCode::Backspace => {
                    self.onboarding.input.pop();
                }
                KeyCode::Char(ch) => {
                    self.onboarding.input.push(ch);
                }
                KeyCode::Enter => {
                    let client_secret = self.onboarding.input.trim().to_string();
                    if client_secret.is_empty() {
                        self.onboarding.message = "Client secret cannot be empty.".to_string();
                    } else {
                        match bw_login_with_apikey(&self.onboarding.bw_client_id, &client_secret) {
                            Ok(_) => {
                                self.onboarding.step = OnboardingStep::BitwardenAuth;
                                self.onboarding.input.clear();
                                self.onboarding.message =
                                    "Bitwarden login successful. Press u to unlock vault.".to_string();
                            }
                            Err(err) => {
                                self.onboarding.message =
                                    format!("Bitwarden login failed: {}", err);
                            }
                        }
                    }
                }
                _ => {}
            },
            OnboardingStep::BitwardenMasterPassword => match code {
                KeyCode::Esc => {
                    self.onboarding.step = OnboardingStep::BitwardenAuth;
                    self.onboarding.input.clear();
                }
                KeyCode::Backspace => {
                    self.onboarding.input.pop();
                }
                KeyCode::Char(ch) => {
                    self.onboarding.input.push(ch);
                }
                KeyCode::Enter => {
                    let password = self.onboarding.input.clone();
                    if password.trim().is_empty() {
                        self.onboarding.message = "Password cannot be empty.".to_string();
                    } else {
                        match bw_unlock_with_password(password.trim()) {
                            Ok(session) => {
                                set_cached_bw_session(Some(session));
                                self.onboarding.step = OnboardingStep::BitwardenAuth;
                                self.onboarding.input.clear();
                                self.onboarding.message =
                                    "Vault unlocked. Press i to continue.".to_string();
                            }
                            Err(err) => {
                                self.onboarding.message =
                                    format!("Bitwarden unlock failed: {}", err);
                            }
                        }
                    }
                }
                _ => {}
            },
            OnboardingStep::BitwardenItemId => match code {
                KeyCode::Esc => {
                    self.onboarding.step = OnboardingStep::BitwardenAuth;
                    self.onboarding.input.clear();
                    self.onboarding.message =
                        "Press c=check, k=API login, u=unlock, i=continue.".to_string();
                }
                KeyCode::Backspace => {
                    self.onboarding.input.pop();
                }
                KeyCode::Char(ch) => {
                    self.onboarding.input.push(ch);
                }
                KeyCode::Enter => {
                    let item_id = self.onboarding.input.trim().to_string();
                    if item_id.is_empty() {
                        self.onboarding.message = "Bitwarden item ID cannot be empty.".to_string();
                        return;
                    }

                    match initialize_bitwarden_config_item(&item_id)
                        .and_then(|_| persist_backend_choice(ConfigBackend::Bitwarden, Some(item_id.clone())))
                    {
                        Ok(_) => self.complete_onboarding("Setup complete: using Bitwarden config"),
                        Err(err) => {
                            self.onboarding.message = format!("Bitwarden setup failed: {}", err);
                        }
                    }
                }
                _ => {}
            },
        }
    }

    fn handle_input_mode(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.input_mode = InputMode::None;
                self.input_buffer.clear();
            }
            KeyCode::Enter => {
                let input = self.input_buffer.trim().to_string();
                match self.input_mode {
                    InputMode::ImportKeyName => {
                        if input.is_empty() {
                            self.status = "Import cancelled".to_string();
                        } else {
                            self.import_state.wallet_name = input;
                            self.input_mode = InputMode::ImportKey;
                            self.input_buffer.clear();
                            return;
                        }
                    }
                    InputMode::ImportKey => {
                        if input.is_empty() {
                            self.status = "Import cancelled".to_string();
                        } else {
                            match keypair_from_secret(&input) {
                                Ok(keypair) => {
                                    let address = keypair.pubkey().to_string();
                                    let mut config = load_den_config();
                                    let wallet_id = next_wallet_id(&config);
                                    let name = if self.import_state.wallet_name.is_empty() {
                                        format!("Wallet {}", config.wallets.len())
                                    } else {
                                        self.import_state.wallet_name.clone()
                                    };
                                    match store_secret_for_wallet(&wallet_id, &input) {
                                        Ok(_) => {
                                            config.wallets.push(WalletEntry {
                                                id: wallet_id.clone(),
                                                name: name.clone(),
                                                address,
                                                has_key: true,
                                                added_at: Some(Utc::now().format("%Y-%m-%d").to_string()),
                                            });
                                            if config.active_wallet.is_none() {
                                                config.active_wallet = Some(wallet_id);
                                            }
                                            let _ = save_den_config(&config);
                                            let msg = format!("Wallet '{}' imported", name);
                                            refresh_wallet_data(self);
                                            self.status = msg;
                                        }
                                        Err(err) => {
                                            self.status = format!("Keychain error: {}", err);
                                        }
                                    }
                                }
                                Err(err) => {
                                    self.status = format!("Invalid key: {}", err);
                                }
                            }
                        }
                    }
                    InputMode::AddWatchOnlyName => {
                        if input.is_empty() {
                            self.status = "Cancelled".to_string();
                        } else {
                            self.import_state.wallet_name = input;
                            self.input_mode = InputMode::AddWatchOnly;
                            self.input_buffer.clear();
                            return;
                        }
                    }
                    InputMode::AddWatchOnly => {
                        if input.is_empty() {
                            self.status = "Cancelled".to_string();
                        } else {
                            let mut config = load_den_config();
                            let wallet_id = next_wallet_id(&config);
                            let name = self.import_state.wallet_name.clone();
                            config.wallets.push(WalletEntry {
                                id: wallet_id.clone(),
                                name: name.clone(),
                                address: input,
                                has_key: false,
                                added_at: Some(Utc::now().format("%Y-%m-%d").to_string()),
                            });
                            if config.active_wallet.is_none() {
                                config.active_wallet = Some(wallet_id);
                            }
                            let _ = save_den_config(&config);
                            let msg = format!("Watch-only wallet '{}' added", name);
                            refresh_wallet_data(self);
                            self.status = msg;
                        }
                    }
                    InputMode::RenameWallet => {
                        if input.is_empty() {
                            self.status = "Rename cancelled".to_string();
                        } else if !self.accounts.is_empty() {
                            let wallet_id = self.accounts[self.selected_account].id.clone();
                            let mut config = load_den_config();
                            if let Some(w) = config.wallets.iter_mut().find(|w| w.id == wallet_id) {
                                w.name = input.clone();
                                let _ = save_den_config(&config);
                                let msg = format!("Renamed to '{}'", input);
                                refresh_wallet_data(self);
                                self.status = msg;
                            }
                        }
                    }
                    InputMode::ConfirmDeleteWallet => {
                        if (input == "y" || input == "yes") && !self.accounts.is_empty() {
                            let selected = &self.accounts[self.selected_account];
                            let wallet_id = selected.id.clone();
                            let wallet_name = selected.name.clone();
                            let had_key = selected.has_key;
                            let mut config = load_den_config();
                            config.wallets.retain(|w| w.id != wallet_id);
                            if config.active_wallet.as_deref() == Some(wallet_id.as_str()) {
                                config.active_wallet =
                                    config.wallets.first().map(|w| w.id.clone());
                            }
                            if had_key {
                                let _ = clear_secret_for_wallet(&wallet_id);
                            }
                            let _ = save_den_config(&config);
                            self.selected_account = 0;
                            self.wallet_detail_index = None;
                            let msg = format!("Wallet '{}' removed", wallet_name);
                            refresh_wallet_data(self);
                            self.status = msg;
                        } else {
                            self.status = "Delete cancelled".to_string();
                        }
                    }
                    InputMode::SignMessage => {
                        if input.is_empty() {
                            self.status = "Sign cancelled".to_string();
                        } else {
                            let config = load_den_config();
                            match active_wallet(&config) {
                                Some(w) if w.has_key => {
                                    match sign_message_with_wallet(&w.id, &input) {
                                        Ok(signature) => {
                                            self.last_signature = signature;
                                            self.status = "Message signed".to_string();
                                        }
                                        Err(err) => {
                                            self.status = format!("Sign failed: {}", err);
                                        }
                                    }
                                }
                                _ => {
                                    self.status = "No signing key available".to_string();
                                }
                            }
                        }
                    }
                    InputMode::AddContactName => {
                        if input.is_empty() {
                            self.status = "Add contact cancelled".to_string();
                        } else {
                            self.import_state.wallet_name = input;
                            self.input_mode = InputMode::AddContactAddress;
                            self.input_buffer.clear();
                            return;
                        }
                    }
                    InputMode::AddContactAddress => {
                        if input.is_empty() {
                            self.status = "Add contact cancelled".to_string();
                        } else {
                            let contact = Contact {
                                name: self.import_state.wallet_name.clone(),
                                address: input,
                                network: "mainnet".to_string(),
                                notes: String::new(),
                            };
                            let name = contact.name.clone();
                            self.contacts.push(contact);
                            let mut file = load_contacts();
                            file.contacts = self.contacts.clone();
                            let _ = save_contacts(&file);
                            self.status = format!("Contact '{}' added", name);
                        }
                    }
                    InputMode::EditContactName => {
                        if input.is_empty() {
                            self.status = "Edit cancelled".to_string();
                        } else {
                            let idx = self.contact_detail_index.unwrap_or(self.selected_contact);
                            if idx < self.contacts.len() {
                                self.contacts[idx].name = input.clone();
                                let mut file = load_contacts();
                                file.contacts = self.contacts.clone();
                                let _ = save_contacts(&file);
                                self.status = format!("Contact updated to '{}'", input);
                            }
                        }
                    }
                    InputMode::EditContactAddress => {
                        if input.is_empty() {
                            self.status = "Edit cancelled".to_string();
                        } else {
                            if let Some(idx) = self.contact_detail_index {
                                if idx < self.contacts.len() {
                                    self.contacts[idx].address = input;
                                    let mut file = load_contacts();
                                    file.contacts = self.contacts.clone();
                                    let _ = save_contacts(&file);
                                    self.status = "Address updated".to_string();
                                }
                            }
                        }
                    }
                    InputMode::EditContactNotes => {
                        if let Some(idx) = self.contact_detail_index {
                            if idx < self.contacts.len() {
                                self.contacts[idx].notes = input;
                                let mut file = load_contacts();
                                file.contacts = self.contacts.clone();
                                let _ = save_contacts(&file);
                                self.status = "Notes updated".to_string();
                            }
                        }
                    }
                    InputMode::ConfirmDeleteContact => {
                        if (input == "y" || input == "yes") && !self.contacts.is_empty() {
                            let idx = self.contact_detail_index.unwrap_or(self.selected_contact);
                            if idx < self.contacts.len() {
                                let name = self.contacts[idx].name.clone();
                                self.contacts.remove(idx);
                                let mut file = load_contacts();
                                file.contacts = self.contacts.clone();
                                let _ = save_contacts(&file);
                                self.contact_detail_index = None;
                                if self.selected_contact >= self.contacts.len() && !self.contacts.is_empty() {
                                    self.selected_contact = self.contacts.len() - 1;
                                }
                                self.status = format!("Contact '{}' deleted", name);
                            }
                        } else {
                            self.status = "Delete cancelled".to_string();
                        }
                    }
                    InputMode::None => {}
                }
                self.input_mode = InputMode::None;
                self.input_buffer.clear();
            }
            KeyCode::Backspace => {
                self.input_buffer.pop();
            }
            KeyCode::Char(ch) => {
                self.input_buffer.push(ch);
            }
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.wallet_detail_index.is_some() || self.contact_detail_index.is_some() {
            return;
        }
        let clamp = |value: isize, max: usize| -> usize {
            if max == 0 {
                return 0;
            }
            let max_index = (max - 1) as isize;
            value.clamp(0, max_index) as usize
        };

        match self.tab {
            Tab::Accounts => {
                let next = self.selected_account as isize + delta;
                self.selected_account = clamp(next, self.accounts.len());
            }
            Tab::Tokens => {
                let next = self.selected_token as isize + delta;
                self.selected_token = clamp(next, self.tokens.len());
            }
            Tab::History => {
                let next = self.selected_history as isize + delta;
                self.selected_history = clamp(next, self.history.len());
            }
            Tab::AddressBook => {
                let next = self.selected_contact as isize + delta;
                self.selected_contact = clamp(next, self.contacts.len());
            }
            _ => {}
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    if handle_cli()? {
        return Ok(());
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>) -> Result<(), Box<dyn Error>> {
    let mut app = build_app();
    let tick_rate = Duration::from_millis(250);

    while !app.should_quit {
        terminal.draw(|frame| ui(frame, &app))?;

        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.on_key(key.code);
                }
            }
        }
    }

    Ok(())
}

fn build_app() -> App {
    ensure_config_exists();
    let mut den_config = load_den_config();
    let needs_onboarding = should_start_onboarding();

    // One-time: migrate API key from keychain to config
    if den_config.network.api_key.is_none() && std::env::var("HELIUS_API_KEY").is_err() {
        if let Ok(key) = load_api_key() {
            den_config.network.api_key = Some(key);
            let _ = save_den_config(&den_config);
        }
    }

    let default_network = match den_config.network.default.as_str() {
        "devnet" => Network::Devnet,
        _ => Network::Mainnet,
    };

    let mut app = App::new_placeholder();
    app.network = default_network;
    app.default_network = den_config.network.default.clone();
    app.config_path_display = config_location_display();
    app.keystore_status = keychain_status_summary(&den_config);
    app.contacts = load_contacts().contacts;
    if needs_onboarding {
        app.start_onboarding();
    }

    refresh_wallet_data(&mut app);

    app
}

fn ui(frame: &mut ratatui::prelude::Frame, app: &App) {
    let area = frame.area();
    render_background(frame, area);
    let footer_height = if area.height >= 24 {
        3
    } else if area.height >= 12 {
        1
    } else {
        0
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(footer_height),
        ])
        .split(area);

    render_header(frame, layout[0], app.tab, area.width, app.network);
    render_body(frame, layout[1], app, area.width);
    if footer_height > 0 {
        render_footer(frame, layout[2], &app.status, footer_height, app.tab, app.wallet_detail_index.is_some() || app.contact_detail_index.is_some());
    }

    if app.onboarding.active {
        render_onboarding_modal(frame, app);
    } else if app.input_mode != InputMode::None {
        render_input_modal(frame, app);
    }
}

fn render_header(
    frame: &mut ratatui::prelude::Frame,
    area: Rect,
    tab: Tab,
    width: u16,
    network: Network,
) {
    if width < 60 {
        let title = format!("Den Wallet | {} | {}", tab.title(), network.label());
        let header = Paragraph::new(title)
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(COLOR_BARK)),
            )
            .style(Style::default().fg(COLOR_ASH));
        frame.render_widget(header, area);
        return;
    }

    let titles = Tab::ALL
        .iter()
        .map(|t| Line::from(Span::styled(t.title(), Style::default().fg(COLOR_ASH))))
        .collect::<Vec<_>>();

    let tabs = Tabs::new(titles)
        .select(tab.index())
        .highlight_style(
            Style::default()
                .fg(COLOR_SOOT)
                .bg(COLOR_FAWN)
                .add_modifier(Modifier::BOLD),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BARK))
                .title(format!(
                    "Den Wallet | {} | {}",
                    tab.title(),
                    network.label()
                )),
        )
        .style(Style::default().fg(COLOR_ASH));

    frame.render_widget(tabs, area);
}

fn render_body(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App, width: u16) {
    if width < 70 {
        render_main(frame, area, app, width);
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(24), Constraint::Min(0)])
        .split(area);

    render_sidebar(frame, layout[0], app.tab);
    render_main(frame, layout[1], app, width);
}

fn render_sidebar(frame: &mut ratatui::prelude::Frame, area: Rect, tab: Tab) {
    let items = Tab::ALL
        .iter()
        .enumerate()
        .map(|(index, t)| {
            let label = format!("{}. {}", index + 1, t.title());
            ListItem::new(Line::from(label))
        })
        .collect::<Vec<_>>();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BARK))
                .title("Sections (1-8)"),
        )
        .highlight_style(
            Style::default()
                .fg(COLOR_SOOT)
                .bg(COLOR_FAWN)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ")
        .style(Style::default().fg(COLOR_ASH));

    frame.render_stateful_widget(list, area, &mut list_state(tab.index()));
}

fn render_main(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App, width: u16) {
    match app.tab {
        Tab::Overview => render_overview(frame, area, app, width),
        Tab::Accounts => render_accounts(frame, area, app),
        Tab::Tokens => render_tokens_view(frame, area, app, width),
        Tab::Send => render_send(frame, area, app),
        Tab::Receive => render_receive(frame, area, app),
        Tab::History => render_history(frame, area, app),
        Tab::AddressBook => render_address_book(frame, area, app),
        Tab::Settings => render_settings(frame, area, app),
    }
}

fn render_overview(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App, width: u16) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(0)])
        .split(area);

    let art = [
        "__         __",
        "/  \\.-\"\"\"-.//  \\",
        "\\    -   -    /",
        " |   o   o   |",
        " \\  .-'''-.  /",
        "  '-\\__Y__/-'",
        "     `---`",
    ];

    let overview = Text::from(
        art.iter()
            .map(|line| Line::from(*line))
            .chain([
                Line::from(""),
                Line::from(format!("Total Balance: {}", app.total_balance)),
                Line::from(format!(
                    "Accounts: {} | Tokens: {}",
                    app.accounts.len(),
                    app.tokens.len()
                )),
            ])
            .collect::<Vec<_>>(),
    );

    let paragraph = Paragraph::new(overview)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BARK))
                .title("Overview"),
        )
        .style(Style::default().fg(COLOR_ASH));

    frame.render_widget(paragraph, layout[0]);

    if width < 90 {
        let bottom = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(layout[1]);
        render_tokens_table(frame, bottom[0], app);
        render_history_list(frame, bottom[1], app);
    } else {
        let bottom = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(layout[1]);
        render_tokens_table(frame, bottom[0], app);
        render_history_list(frame, bottom[1], app);
    }
}

fn render_accounts(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App) {
    if let Some(index) = app.wallet_detail_index {
        render_wallet_detail(frame, area, app, index);
        return;
    }

    let rows = app.accounts.iter().map(|account| {
        let marker = if account.is_active { "*" } else { " " };
        let wallet_type = if account.has_key { "Full" } else { "Watch" };
        Row::new(vec![
            format!("{} {}", marker, account.name),
            short_address(&account.address),
            account.balance.clone(),
            wallet_type.to_string(),
        ])
    });

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(30),
            Constraint::Percentage(20),
        ],
    )
    .header(
        Row::new(vec!["Name", "Address", "Balance", "Type"]).style(
            Style::default()
                .fg(COLOR_FAWN)
                .bg(COLOR_BARK)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(COLOR_BARK))
            .title("Wallets [Enter:details a:add w:watch e:rename d:delete]"),
    )
    .row_highlight_style(
        Style::default()
            .fg(COLOR_SOOT)
            .bg(COLOR_FAWN)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("> ");

    frame.render_stateful_widget(table, area, &mut table_state(app.selected_account));
}

fn render_wallet_detail(
    frame: &mut ratatui::prelude::Frame,
    area: Rect,
    app: &App,
    index: usize,
) {
    let account = match app.accounts.get(index) {
        Some(a) => a,
        None => {
            let msg = Paragraph::new("Wallet not found")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(COLOR_BARK))
                        .title("Wallet Detail"),
                )
                .style(Style::default().fg(COLOR_EMBER));
            frame.render_widget(msg, area);
            return;
        }
    };

    let wallet_type = if account.has_key {
        "Full (signing key stored)"
    } else {
        "Watch-only"
    };
    let active_status = if account.is_active {
        "Yes"
    } else {
        "No"
    };
    let added_display = account
        .added_at
        .as_deref()
        .unwrap_or("Unknown");

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(14), Constraint::Min(0)])
        .split(area);

    let info = Text::from(vec![
        Line::from(vec![
            Span::styled("  Name:     ", Style::default().fg(COLOR_STONE)),
            Span::styled(&account.name, Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Address:  ", Style::default().fg(COLOR_STONE)),
            Span::styled(&account.address, Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Balance:  ", Style::default().fg(COLOR_STONE)),
            Span::styled(&account.balance, Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Type:     ", Style::default().fg(COLOR_STONE)),
            Span::styled(wallet_type, Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Active:   ", Style::default().fg(COLOR_STONE)),
            Span::styled(
                active_status,
                Style::default().fg(if account.is_active {
                    COLOR_MOSS
                } else {
                    COLOR_STONE
                }),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Added:    ", Style::default().fg(COLOR_STONE)),
            Span::styled(added_display, Style::default().fg(COLOR_ASH)),
        ]),
    ]);

    let paragraph = Paragraph::new(info)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BARK))
                .title(format!("Wallet: {}", account.name)),
        )
        .style(Style::default().fg(COLOR_ASH));

    frame.render_widget(paragraph, layout[0]);

    let hints = Text::from(vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Enter", Style::default().fg(COLOR_FAWN).add_modifier(Modifier::BOLD)),
            Span::styled("  Set as active wallet", Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(vec![
            Span::styled("  e", Style::default().fg(COLOR_FAWN).add_modifier(Modifier::BOLD)),
            Span::styled("      Rename wallet", Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(vec![
            Span::styled("  d", Style::default().fg(COLOR_FAWN).add_modifier(Modifier::BOLD)),
            Span::styled("      Delete wallet", Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(vec![
            Span::styled("  Esc", Style::default().fg(COLOR_FAWN).add_modifier(Modifier::BOLD)),
            Span::styled("    Back to wallet list", Style::default().fg(COLOR_ASH)),
        ]),
    ]);

    let actions = Paragraph::new(hints)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BARK))
                .title("Actions"),
        )
        .style(Style::default().fg(COLOR_ASH));

    frame.render_widget(actions, layout[1]);
}

fn render_tokens_view(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App, width: u16) {
    if width < 90 {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);
        render_tokens_table(frame, layout[0], app);
        render_token_chart(frame, layout[1], app);
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);
    render_tokens_table(frame, layout[0], app);
    render_token_chart(frame, layout[1], app);
}

fn render_tokens_table(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App) {
    let rows = app.tokens.iter().map(|token| {
        Row::new(vec![
            ratatui::widgets::Cell::from(token.symbol.clone()),
            ratatui::widgets::Cell::from(token.balance.clone()),
            ratatui::widgets::Cell::from(token.value.clone()),
        ])
    });

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Percentage(35),
            Constraint::Percentage(35),
        ],
    )
    .header(
        Row::new(vec!["Token", "Balance", "Value"]).style(
            Style::default()
                .fg(COLOR_FAWN)
                .bg(COLOR_BARK)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(COLOR_BARK))
            .title("Tokens"),
    )
    .row_highlight_style(
        Style::default()
            .fg(COLOR_SOOT)
            .bg(COLOR_FAWN)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("> ");

    frame.render_stateful_widget(table, area, &mut table_state(app.selected_token));
}

fn render_token_chart(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App) {
    let token = app.tokens.get(app.selected_token).or(app.tokens.first());
    let (title, history) = match token {
        Some(token) => (
            format!("{} price (24h)", token.symbol),
            token.history.as_slice(),
        ),
        None => ("Token price (24h)".to_string(), &[][..]),
    };

    let data = history
        .iter()
        .enumerate()
        .map(|(index, value)| (index as f64, *value))
        .collect::<Vec<(f64, f64)>>();

    let (min, max) = series_bounds(history);
    let x_max = history.len().saturating_sub(1).max(1) as f64;

    let dataset = Dataset::default()
        .name("price")
        .marker(ratatui::symbols::Marker::Dot)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(COLOR_FAWN))
        .data(&data);

    let chart = Chart::new(vec![dataset])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BARK))
                .title(title),
        )
        .x_axis(
            Axis::default()
                .title("time")
                .style(Style::default().fg(COLOR_STONE))
                .bounds([0.0, x_max])
                .labels([
                    Span::styled("24h", Style::default().fg(COLOR_STONE)),
                    Span::styled("now", Style::default().fg(COLOR_STONE)),
                ]),
        )
        .y_axis(
            Axis::default()
                .title("price")
                .style(Style::default().fg(COLOR_STONE))
                .bounds([min, max])
                .labels([
                    Span::styled(format!("{:.2}", min), Style::default().fg(COLOR_STONE)),
                    Span::styled(format!("{:.2}", max), Style::default().fg(COLOR_STONE)),
                ]),
        );

    frame.render_widget(chart, area);
}

fn render_history(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App) {
    render_history_list(frame, area, app);
}

fn render_history_list(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App) {
    let items = app
        .history
        .iter()
        .map(|tx| {
            ListItem::new(Line::from(format!(
                "{}  {}  {}",
                tx.time, tx.summary, tx.amount
            )))
        })
        .collect::<Vec<_>>();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BARK))
                .title("Recent Activity"),
        )
        .highlight_style(
            Style::default()
                .fg(COLOR_SOOT)
                .bg(COLOR_FAWN)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ")
        .style(Style::default().fg(COLOR_ASH));

    frame.render_stateful_widget(list, area, &mut list_state(app.selected_history));
}

fn render_address_book(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App) {
    if let Some(index) = app.contact_detail_index {
        render_contact_detail(frame, area, app, index);
        return;
    }

    let items = app
        .contacts
        .iter()
        .map(|contact| {
            let line = format!(
                "{}  {}  [{}]",
                contact.name,
                short_address(&contact.address),
                contact.network
            );
            ListItem::new(Line::from(line))
        })
        .collect::<Vec<_>>();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BARK))
                .title("Address Book [Enter:details a:add e:edit d:delete]"),
        )
        .highlight_style(
            Style::default()
                .fg(COLOR_SOOT)
                .bg(COLOR_FAWN)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ")
        .style(Style::default().fg(COLOR_ASH));

    frame.render_stateful_widget(list, area, &mut list_state(app.selected_contact));
}

fn render_contact_detail(
    frame: &mut ratatui::prelude::Frame,
    area: Rect,
    app: &App,
    index: usize,
) {
    let contact = match app.contacts.get(index) {
        Some(c) => c,
        None => {
            let msg = Paragraph::new("Contact not found")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(COLOR_BARK))
                        .title("Contact Detail"),
                )
                .style(Style::default().fg(COLOR_EMBER));
            frame.render_widget(msg, area);
            return;
        }
    };

    let notes_display = if contact.notes.is_empty() {
        "(none)"
    } else {
        &contact.notes
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(12), Constraint::Min(0)])
        .split(area);

    let info = Text::from(vec![
        Line::from(vec![
            Span::styled("  Name:     ", Style::default().fg(COLOR_STONE)),
            Span::styled(&contact.name, Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Address:  ", Style::default().fg(COLOR_STONE)),
            Span::styled(&contact.address, Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Network:  ", Style::default().fg(COLOR_STONE)),
            Span::styled(&contact.network, Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Notes:    ", Style::default().fg(COLOR_STONE)),
            Span::styled(notes_display, Style::default().fg(COLOR_ASH)),
        ]),
    ]);

    let paragraph = Paragraph::new(info)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BARK))
                .title(format!("Contact: {}", contact.name)),
        )
        .style(Style::default().fg(COLOR_ASH));

    frame.render_widget(paragraph, layout[0]);

    let hints = Text::from(vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  e", Style::default().fg(COLOR_FAWN).add_modifier(Modifier::BOLD)),
            Span::styled("      Edit name", Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(vec![
            Span::styled("  a", Style::default().fg(COLOR_FAWN).add_modifier(Modifier::BOLD)),
            Span::styled("      Edit address", Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(vec![
            Span::styled("  o", Style::default().fg(COLOR_FAWN).add_modifier(Modifier::BOLD)),
            Span::styled("      Edit notes", Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(vec![
            Span::styled("  d", Style::default().fg(COLOR_FAWN).add_modifier(Modifier::BOLD)),
            Span::styled("      Delete contact", Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(vec![
            Span::styled("  Esc", Style::default().fg(COLOR_FAWN).add_modifier(Modifier::BOLD)),
            Span::styled("    Back to contact list", Style::default().fg(COLOR_ASH)),
        ]),
    ]);

    let actions = Paragraph::new(hints)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BARK))
                .title("Actions"),
        )
        .style(Style::default().fg(COLOR_ASH));

    frame.render_widget(actions, layout[1]);
}

fn render_send(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App) {
    if let Some(acc) = app.accounts.iter().find(|a| a.is_active) {
        if !acc.has_key {
            let notice = Paragraph::new(
                "Watch-only wallet -- signing not available.\nSwitch to a full wallet to send.",
            )
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(COLOR_BARK))
                    .title("Send"),
            )
            .style(Style::default().fg(COLOR_EMBER));
            frame.render_widget(notice, area);
            return;
        }
    }

    let (account_name, account_address) = active_account(app);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Min(0),
        ])
        .split(area);

    let fields = Text::from(vec![
        Line::from(format!("From: {} ({})", account_name, account_address)),
        Line::from("To:    [paste address or pick contact]"),
        Line::from("Asset: SOL"),
        Line::from("Amount: 0.00"),
    ]);

    let details = Text::from(vec![
        Line::from("Network: Solana mainnet"),
        Line::from("Fee: 0.000005 SOL"),
        Line::from("Max: 0.00 SOL"),
    ]);

    let actions = Paragraph::new("[Enter] Review & Send   [Esc] Cancel")
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BARK))
                .title("Actions"),
        )
        .style(Style::default().fg(COLOR_ASH));

    frame.render_widget(
        Paragraph::new(fields)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(COLOR_BARK))
                    .title("Send"),
            )
            .style(Style::default().fg(COLOR_ASH)),
        layout[0],
    );
    frame.render_widget(
        Paragraph::new(details)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(COLOR_BARK))
                    .title("Details"),
            )
            .style(Style::default().fg(COLOR_ASH)),
        layout[1],
    );
    frame.render_widget(actions, layout[2]);
}

fn render_receive(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App) {
    let (account_name, account_address) = active_account(app);

    let receive = Text::from(vec![
        Line::from(format!("Account: {}", account_name)),
        Line::from(format!("Address: {}", account_address)),
        Line::from("Memo: (optional)"),
        Line::from("QR: [placeholder]"),
    ]);

    let paragraph = Paragraph::new(receive)
        .alignment(Alignment::Left)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BARK))
                .title("Receive"),
        )
        .style(Style::default().fg(COLOR_ASH));

    frame.render_widget(paragraph, area);
}

fn render_settings(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App) {
    let active_name = app
        .accounts
        .iter()
        .find(|a| a.is_active)
        .map(|a| a.name.as_str())
        .unwrap_or("None");
    let wallet_count = app.accounts.len();
    let full_count = app.accounts.iter().filter(|a| a.has_key).count();
    let watch_count = wallet_count - full_count;

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Min(0),
        ])
        .split(area);

    let network_section = Text::from(vec![
        Line::from(vec![
            Span::styled("  Network:    ", Style::default().fg(COLOR_STONE)),
            Span::styled(app.network.label(), Style::default().fg(COLOR_ASH)),
            Span::styled("  (n to toggle)", Style::default().fg(COLOR_STONE)),
        ]),
        Line::from(vec![
            Span::styled("  Default:    ", Style::default().fg(COLOR_STONE)),
            Span::styled(&app.default_network, Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(vec![
            Span::styled("  API Key:    ", Style::default().fg(COLOR_STONE)),
            Span::styled(&app.api_key_status, Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(vec![
            Span::styled("  Config:     ", Style::default().fg(COLOR_STONE)),
            Span::styled(&app.config_path_display, Style::default().fg(COLOR_ASH)),
        ]),
    ]);

    let wallet_section = Text::from(vec![
        Line::from(vec![
            Span::styled("  Active:     ", Style::default().fg(COLOR_STONE)),
            Span::styled(active_name, Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(vec![
            Span::styled("  Address:    ", Style::default().fg(COLOR_STONE)),
            Span::styled(&app.wallet_address, Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(vec![
            Span::styled("  Wallets:    ", Style::default().fg(COLOR_STONE)),
            Span::styled(
                format!("{} total ({} full, {} watch-only)", wallet_count, full_count, watch_count),
                Style::default().fg(COLOR_ASH),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Last sig:   ", Style::default().fg(COLOR_STONE)),
            Span::styled(&app.last_signature, Style::default().fg(COLOR_ASH)),
        ]),
    ]);

    let shortcuts = Text::from(vec![
        Line::from(vec![
            Span::styled("  n", Style::default().fg(COLOR_FAWN).add_modifier(Modifier::BOLD)),
            Span::styled("  Toggle network", Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(vec![
            Span::styled("  r", Style::default().fg(COLOR_FAWN).add_modifier(Modifier::BOLD)),
            Span::styled("  Refresh data", Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(vec![
            Span::styled("  i", Style::default().fg(COLOR_FAWN).add_modifier(Modifier::BOLD)),
            Span::styled("  Import wallet", Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(vec![
            Span::styled("  s", Style::default().fg(COLOR_FAWN).add_modifier(Modifier::BOLD)),
            Span::styled("  Sign message", Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(vec![
            Span::styled("  o", Style::default().fg(COLOR_FAWN).add_modifier(Modifier::BOLD)),
            Span::styled("  Run setup wizard (Settings tab)", Style::default().fg(COLOR_ASH)),
        ]),
        Line::from(vec![
            Span::styled("  2", Style::default().fg(COLOR_FAWN).add_modifier(Modifier::BOLD)),
            Span::styled("  Manage wallets (Accounts tab)", Style::default().fg(COLOR_ASH)),
        ]),
    ]);

    frame.render_widget(
        Paragraph::new(network_section)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(COLOR_BARK))
                    .title("Configuration"),
            )
            .style(Style::default().fg(COLOR_ASH)),
        layout[0],
    );
    frame.render_widget(
        Paragraph::new(wallet_section)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(COLOR_BARK))
                    .title("Wallets"),
            )
            .style(Style::default().fg(COLOR_ASH)),
        layout[1],
    );
    frame.render_widget(
        Paragraph::new(shortcuts)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(COLOR_BARK))
                    .title("Shortcuts"),
            )
            .style(Style::default().fg(COLOR_ASH)),
        layout[2],
    );
}

fn footer_nav_text(tab: Tab, in_detail: bool) -> &'static str {
    match tab {
        Tab::Accounts if in_detail => "Enter:activate | e:rename | d:delete | Esc:back | q:quit",
        Tab::Accounts => "Enter:details | a:add | w:watch | e:rename | d:delete | r:refresh | q:quit",
        Tab::AddressBook if in_detail => "e:name | a:address | o:notes | d:delete | Esc:back",
        Tab::AddressBook => "Enter:details | a:add | e:edit | d:delete | q:quit",
        _ => "1-8 | up/down | n:network | i:import | s:sign | r:refresh | q:quit",
    }
}

fn render_footer(
    frame: &mut ratatui::prelude::Frame,
    area: Rect,
    status: &str,
    height: u16,
    tab: Tab,
    in_detail: bool,
) {
    let nav_text = footer_nav_text(tab, in_detail);
    if height == 1 {
        let content = format!("{} | {}", nav_text, status);
        let footer = Paragraph::new(content)
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(COLOR_BARK)),
            )
            .style(Style::default().fg(COLOR_ASH));
        frame.render_widget(footer, area);
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let nav = Paragraph::new(nav_text)
        .alignment(Alignment::Center)
        .style(Style::default().fg(COLOR_ASH));
    let status_line = Paragraph::new(status)
        .alignment(Alignment::Center)
        .style(status_style(status));

    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(COLOR_BARK)),
        area,
    );
    frame.render_widget(nav, layout[0]);
    frame.render_widget(status_line, layout[1]);
}

fn list_state(selected: usize) -> ratatui::widgets::ListState {
    let mut state = ratatui::widgets::ListState::default();
    state.select(Some(selected));
    state
}

fn table_state(selected: usize) -> ratatui::widgets::TableState {
    let mut state = ratatui::widgets::TableState::default();
    state.select(Some(selected));
    state
}

fn active_account(app: &App) -> (String, String) {
    app.accounts
        .iter()
        .find(|a| a.is_active)
        .map(|a| (a.name.clone(), short_address(&a.address)))
        .unwrap_or_else(|| ("None".to_string(), "Unset".to_string()))
}

fn render_onboarding_modal(frame: &mut ratatui::prelude::Frame, app: &App) {
    let area = frame.area();
    let modal_width = area.width.saturating_sub(6).min(86).max(24);
    let modal_height = 11u16;
    let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
    let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
    let modal = Rect::new(x, y, modal_width, modal_height);

    let (prompt, input_line, hints): (String, String, Vec<String>) = match app.onboarding.step {
        OnboardingStep::ChooseBackend => (
            "Choose configuration backend:".to_string(),
            "".to_string(),
            vec![
                "1) This Mac (local file)".to_string(),
                "2) Bitwarden (sync across machines)".to_string(),
                "q) Quit".to_string(),
            ],
        ),
        OnboardingStep::BitwardenAuth => (
            "Bitwarden auth required before selecting config item.".to_string(),
            "".to_string(),
            vec![
                "c) Check status".to_string(),
                "k) Login with API key".to_string(),
                "u) Unlock with master password".to_string(),
                "i) Continue to item ID once unlocked".to_string(),
                "Esc) Back".to_string(),
            ],
        ),
        OnboardingStep::BitwardenApiKeyId => (
            "Enter Bitwarden API client ID:".to_string(),
            app.onboarding.input.clone(),
            vec!["Enter to continue, Esc to cancel".to_string()],
        ),
        OnboardingStep::BitwardenApiKeySecret => (
            "Enter Bitwarden API client secret:".to_string(),
            "*".repeat(app.onboarding.input.len()),
            vec!["Enter to submit, Esc to cancel".to_string()],
        ),
        OnboardingStep::BitwardenMasterPassword => (
            "Enter Bitwarden master password:".to_string(),
            "*".repeat(app.onboarding.input.len()),
            vec!["Enter to unlock, Esc to cancel".to_string()],
        ),
        OnboardingStep::BitwardenItemId => (
            "Enter Bitwarden config item ID:".to_string(),
            app.onboarding.input.clone(),
            vec![
                "Enter to continue, Esc to go back".to_string(),
            ],
        ),
    };

    let mut lines = vec![
        Line::from(prompt),
        Line::from(""),
        Line::from(input_line),
        Line::from(""),
    ];
    for hint in hints {
        lines.push(Line::from(hint));
    }
    if !app.onboarding.message.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(app.onboarding.message.clone()));
    }

    let paragraph = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BARK))
                .title("Setup"),
        )
        .style(Style::default().fg(COLOR_ASH));

    frame.render_widget(paragraph, modal);
}

fn render_input_modal(frame: &mut ratatui::prelude::Frame, app: &App) {
    let area = frame.area();
    let modal_width = area.width.saturating_sub(8).min(80).max(20);
    let modal_height = 7u16;
    let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
    let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
    let modal = Rect::new(x, y, modal_width, modal_height);

    let delete_name = app
        .accounts
        .get(app.selected_account)
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "?".to_string());
    let delete_prompt = format!("Delete '{}'? Type 'y' to confirm:", delete_name);

    let contact_delete_name = {
        let idx = app.contact_detail_index.unwrap_or(app.selected_contact);
        app.contacts
            .get(idx)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "?".to_string())
    };
    let contact_delete_prompt = format!("Delete '{}'? Type 'y' to confirm:", contact_delete_name);

    let (title, prompt, display): (&str, String, String) = match app.input_mode {
        InputMode::ImportKeyName => (
            "Add Wallet",
            "Enter a name for this wallet:".to_string(),
            app.input_buffer.clone(),
        ),
        InputMode::ImportKey => {
            let masked = "*".repeat(app.input_buffer.len());
            (
                "Add Wallet",
                "Paste secret key and press Enter:".to_string(),
                masked,
            )
        }
        InputMode::AddWatchOnlyName => (
            "Add Watch-Only",
            "Enter a name for this wallet:".to_string(),
            app.input_buffer.clone(),
        ),
        InputMode::AddWatchOnly => (
            "Add Watch-Only",
            "Paste the public address:".to_string(),
            app.input_buffer.clone(),
        ),
        InputMode::RenameWallet => (
            "Rename Wallet",
            "Enter new name:".to_string(),
            app.input_buffer.clone(),
        ),
        InputMode::ConfirmDeleteWallet => (
            "Delete Wallet",
            delete_prompt,
            app.input_buffer.clone(),
        ),
        InputMode::SignMessage => (
            "Sign Message",
            "Enter message and press Enter:".to_string(),
            app.input_buffer.clone(),
        ),
        InputMode::AddContactName => (
            "Add Contact",
            "Enter contact name:".to_string(),
            app.input_buffer.clone(),
        ),
        InputMode::AddContactAddress => (
            "Add Contact",
            "Enter wallet address:".to_string(),
            app.input_buffer.clone(),
        ),
        InputMode::EditContactName => (
            "Edit Contact",
            "Enter new name:".to_string(),
            app.input_buffer.clone(),
        ),
        InputMode::EditContactAddress => (
            "Edit Contact",
            "Enter new address:".to_string(),
            app.input_buffer.clone(),
        ),
        InputMode::EditContactNotes => (
            "Edit Notes",
            "Enter notes (or leave empty to clear):".to_string(),
            app.input_buffer.clone(),
        ),
        InputMode::ConfirmDeleteContact => (
            "Delete Contact",
            contact_delete_prompt,
            app.input_buffer.clone(),
        ),
        InputMode::None => ("", String::new(), String::new()),
    };

    let content = Text::from(vec![
        Line::from(prompt.as_str().to_string()),
        Line::from(""),
        Line::from(display),
        Line::from(""),
        Line::from("Esc to cancel"),
    ]);

    let paragraph = Paragraph::new(content)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BARK))
                .title(title),
        )
        .style(Style::default().fg(COLOR_ASH));

    frame.render_widget(paragraph, modal);
}

fn status_style(message: &str) -> Style {
    let lower = message.to_ascii_lowercase();
    if lower.contains("error") || lower.contains("failed") || lower.contains("bad") {
        Style::default().fg(COLOR_EMBER)
    } else if lower.contains("stored")
        || lower.contains("signed")
        || lower.contains("set to")
        || lower.contains("live data")
        || lower.contains("imported")
        || lower.contains("added")
        || lower.contains("switched")
        || lower.contains("renamed")
        || lower.contains("removed")
        || lower.contains("updated")
        || lower.contains("deleted")
    {
        Style::default().fg(COLOR_MOSS)
    } else {
        Style::default().fg(COLOR_STONE)
    }
}

fn render_background(frame: &mut ratatui::prelude::Frame, area: Rect) {
    let background = Block::default().style(Style::default().bg(COLOR_SOOT));
    frame.render_widget(background, area);
}

fn seeded_series(seed: &str, length: usize) -> Vec<f64> {
    let mut state: u64 = 1469598103934665603;
    for byte in seed.as_bytes() {
        state ^= *byte as u64;
        state = state.wrapping_mul(1099511628211);
    }

    let mut values = Vec::with_capacity(length);
    let mut current = (state % 1000) as f64 / 10.0 + 10.0;
    for _ in 0..length {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let change = ((state >> 33) as i64 % 11) as f64 / 10.0 - 0.5;
        current = (current + change).max(1.0);
        values.push(current);
    }

    values
}

fn series_bounds(values: &[f64]) -> (f64, f64) {
    if values.is_empty() {
        return (0.0, 1.0);
    }

    let mut min = f64::MAX;
    let mut max = f64::MIN;
    for value in values {
        if *value < min {
            min = *value;
        }
        if *value > max {
            max = *value;
        }
    }

    if (max - min).abs() < f64::EPSILON {
        return (min - 1.0, max + 1.0);
    }

    let padding = (max - min) * 0.08;
    (min - padding, max + padding)
}

fn handle_cli() -> Result<bool, Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    if let Some(arg) = args.next() {
        match arg.as_str() {
            "--import" => {
                let secret =
                    std::env::var("DEN_SECRET_KEY").map_err(|_| "DEN_SECRET_KEY is not set")?;
                let keypair = keypair_from_secret(&secret)?;
                let address = keypair.pubkey().to_string();
                ensure_config_exists();
                let mut config = load_den_config();
                let wallet_id = next_wallet_id(&config);
                store_secret_for_wallet(&wallet_id, &secret)?;
                config.wallets.push(WalletEntry {
                    id: wallet_id.clone(),
                    name: "Imported".to_string(),
                    address: address.clone(),
                    has_key: true,
                    added_at: Some(Utc::now().format("%Y-%m-%d").to_string()),
                });
                if config.active_wallet.is_none() {
                    config.active_wallet = Some(wallet_id.clone());
                }
                save_den_config(&config)?;
                println!("Key imported as '{}' ({}): {}", "Imported", wallet_id, short_address(&address));
                return Ok(true);
            }
            "--add-wallet" => {
                let name = args.next().ok_or("Usage: den --add-wallet <name>")?;
                let secret =
                    std::env::var("DEN_SECRET_KEY").map_err(|_| "DEN_SECRET_KEY is not set")?;
                let keypair = keypair_from_secret(&secret)?;
                let address = keypair.pubkey().to_string();
                ensure_config_exists();
                let mut config = load_den_config();
                let wallet_id = next_wallet_id(&config);
                store_secret_for_wallet(&wallet_id, &secret)?;
                config.wallets.push(WalletEntry {
                    id: wallet_id.clone(),
                    name: name.clone(),
                    address: address.clone(),
                    has_key: true,
                    added_at: Some(Utc::now().format("%Y-%m-%d").to_string()),
                });
                if config.active_wallet.is_none() {
                    config.active_wallet = Some(wallet_id.clone());
                }
                save_den_config(&config)?;
                println!("Added wallet '{}' ({}): {}", name, wallet_id, short_address(&address));
                return Ok(true);
            }
            "--add-watch" => {
                let name = args.next().ok_or("Usage: den --add-watch <name> <address>")?;
                let address = args.next().ok_or("Usage: den --add-watch <name> <address>")?;
                ensure_config_exists();
                let mut config = load_den_config();
                let wallet_id = next_wallet_id(&config);
                config.wallets.push(WalletEntry {
                    id: wallet_id.clone(),
                    name: name.clone(),
                    address: address.clone(),
                    has_key: false,
                    added_at: Some(Utc::now().format("%Y-%m-%d").to_string()),
                });
                if config.active_wallet.is_none() {
                    config.active_wallet = Some(wallet_id.clone());
                }
                save_den_config(&config)?;
                println!("Added watch-only '{}' ({}): {}", name, wallet_id, short_address(&address));
                return Ok(true);
            }
            "--list-wallets" => {
                ensure_config_exists();
                let config = load_den_config();
                if config.wallets.is_empty() {
                    println!("No wallets configured.");
                } else {
                    let active = config.active_wallet.as_deref().unwrap_or("");
                    for w in &config.wallets {
                        let marker = if w.id == active { "*" } else { " " };
                        let wtype = if w.has_key { "full" } else { "watch" };
                        println!("{} {} ({}) [{}] {}", marker, w.name, w.id, wtype, short_address(&w.address));
                    }
                }
                return Ok(true);
            }
            "--remove-wallet" => {
                let target = args.next().ok_or("Usage: den --remove-wallet <name-or-id>")?;
                ensure_config_exists();
                let mut config = load_den_config();
                let idx = config
                    .wallets
                    .iter()
                    .position(|w| w.id == target || w.name == target)
                    .ok_or(format!("Wallet '{}' not found", target))?;
                let removed = config.wallets.remove(idx);
                if removed.has_key {
                    let _ = clear_secret_for_wallet(&removed.id);
                }
                if config.active_wallet.as_deref() == Some(&removed.id) {
                    config.active_wallet = config.wallets.first().map(|w| w.id.clone());
                }
                save_den_config(&config)?;
                println!("Removed wallet '{}' ({}).", removed.name, removed.id);
                return Ok(true);
            }
            "--switch-wallet" => {
                let target = args.next().ok_or("Usage: den --switch-wallet <name-or-id>")?;
                ensure_config_exists();
                let mut config = load_den_config();
                let wallet = config
                    .wallets
                    .iter()
                    .find(|w| w.id == target || w.name == target)
                    .ok_or(format!("Wallet '{}' not found", target))?;
                let wallet_id = wallet.id.clone();
                let wallet_name = wallet.name.clone();
                config.active_wallet = Some(wallet_id);
                save_den_config(&config)?;
                println!("Active wallet set to '{}'.", wallet_name);
                return Ok(true);
            }
            "--rename-wallet" => {
                let target = args.next().ok_or("Usage: den --rename-wallet <name-or-id> <new-name>")?;
                let new_name = args.next().ok_or("Usage: den --rename-wallet <name-or-id> <new-name>")?;
                ensure_config_exists();
                let mut config = load_den_config();
                let wallet = config
                    .wallets
                    .iter_mut()
                    .find(|w| w.id == target || w.name == target)
                    .ok_or(format!("Wallet '{}' not found", target))?;
                wallet.name = new_name.clone();
                save_den_config(&config)?;
                println!("Wallet renamed to '{}'.", new_name);
                return Ok(true);
            }
            "--clear" => {
                ensure_config_exists();
                let mut config = load_den_config();
                let target = args.next();
                let wallet_id = if let Some(t) = target {
                    config
                        .wallets
                        .iter()
                        .find(|w| w.id == t || w.name == t)
                        .map(|w| w.id.clone())
                } else {
                    config.active_wallet.clone()
                };
                match wallet_id {
                    Some(id) => {
                        let wallet = config.wallets.iter().find(|w| w.id == id);
                        match wallet {
                            Some(w) if w.has_key => {
                                let name = w.name.clone();
                                clear_secret_for_wallet(&id)?;
                                if let Some(entry) = config.wallets.iter_mut().find(|e| e.id == id) {
                                    entry.has_key = false;
                                }
                                save_den_config(&config)?;
                                println!("Key removed for wallet '{}'. Now watch-only.", name);
                            }
                            Some(w) => println!("Wallet '{}' is already watch-only.", w.name),
                            None => println!("Wallet not found."),
                        }
                    }
                    None => println!("No wallet found to clear."),
                }
                return Ok(true);
            }
            "--set-api-key" => {
                let key = args.next().ok_or("Usage: den --set-api-key <KEY>")?;
                ensure_config_exists();
                let mut config = load_den_config();
                config.network.api_key = Some(key);
                save_den_config(&config)?;
                println!("API key saved to config.");
                return Ok(true);
            }
            "--clear-api-key" => {
                ensure_config_exists();
                let mut config = load_den_config();
                config.network.api_key = None;
                save_den_config(&config)?;
                let _ = clear_api_key();
                println!("API key removed.");
                return Ok(true);
            }
            "--set-network" => {
                let net = args.next().ok_or("Usage: den --set-network <mainnet|devnet>")?;
                match net.as_str() {
                    "mainnet" | "devnet" => {
                        ensure_config_exists();
                        let mut config = load_den_config();
                        config.network.default = net;
                        save_den_config(&config)?;
                        println!("Default network saved to config.");
                    }
                    _ => return Err("Network must be 'mainnet' or 'devnet'".into()),
                }
                return Ok(true);
            }
            "--migrate-config-to-bitwarden" => {
                let force = matches!(args.next().as_deref(), Some("--force"));
                let location = migrate_local_config_to_bitwarden(force)?;
                println!("Migrated local config to {}.", location);
                return Ok(true);
            }
            "--config-path" => {
                println!("{}", config_location_display());
                return Ok(true);
            }
            "--status" => {
                ensure_config_exists();
                let config = load_den_config();
                println!("Den Wallet Status");
                println!("  Config: {}", config_location_display());
                println!("  Default network: {}", config.network.default);
                println!("  {}", api_key_status(&config));
                println!("  Wallets: {}", config.wallets.len());
                let active_name = active_wallet(&config)
                    .map(|w| w.name.as_str())
                    .unwrap_or("none");
                println!("  Active: {}", active_name);
                for w in &config.wallets {
                    let marker = if config.active_wallet.as_deref() == Some(w.id.as_str()) {
                        "*"
                    } else {
                        " "
                    };
                    let wtype = if w.has_key { "full" } else { "watch" };
                    println!("    {} {} [{}] {}", marker, w.name, wtype, short_address(&w.address));
                }
                return Ok(true);
            }
            "--list-contacts" => {
                let file = load_contacts();
                if file.contacts.is_empty() {
                    println!("No contacts.");
                } else {
                    for c in &file.contacts {
                        let notes = if c.notes.is_empty() {
                            String::new()
                        } else {
                            format!(" -- {}", c.notes)
                        };
                        println!(
                            "  {} [{}] {}{}",
                            c.name,
                            c.network,
                            short_address(&c.address),
                            notes
                        );
                    }
                }
                return Ok(true);
            }
            "--export-contacts" => {
                let file = load_contacts();
                let json = serde_json::to_string_pretty(&file)?;
                match args.next() {
                    Some(path) => {
                        std::fs::write(&path, &json)?;
                        println!("Exported {} contacts to {}", file.contacts.len(), path);
                    }
                    None => {
                        println!("{}", json);
                    }
                }
                return Ok(true);
            }
            "--import-contacts" => {
                let path = args.next().ok_or("Usage: den --import-contacts <file>")?;
                let contents = std::fs::read_to_string(&path)?;
                let incoming: ContactsFile = serde_json::from_str(&contents)?;
                let mut file = load_contacts();
                let mut added = 0u32;
                let mut skipped = 0u32;
                for contact in incoming.contacts {
                    if file.contacts.iter().any(|c| c.address == contact.address) {
                        skipped += 1;
                    } else {
                        file.contacts.push(contact);
                        added += 1;
                    }
                }
                save_contacts(&file)?;
                println!(
                    "Imported {} contacts, skipped {} duplicates.",
                    added, skipped
                );
                return Ok(true);
            }
            "--help" => {
                println!("Den Wallet CLI");
                println!();
                println!("Wallet Management:");
                println!("  --add-wallet NAME       Import key from DEN_SECRET_KEY with name");
                println!("  --add-watch NAME ADDR   Add a watch-only wallet");
                println!("  --list-wallets          List all wallets");
                println!("  --switch-wallet NAME    Set active wallet by name or ID");
                println!("  --rename-wallet NAME NEW  Rename a wallet");
                println!("  --remove-wallet NAME    Remove a wallet");
                println!("  --import                Import key from DEN_SECRET_KEY (legacy)");
                println!("  --clear [NAME]          Remove private key (active or named)");
                println!();
                println!("Contacts:");
                println!("  --list-contacts         List all contacts");
                println!("  --export-contacts [FILE] Export contacts as JSON (stdout or file)");
                println!("  --import-contacts FILE  Import contacts from JSON, skip duplicates");
                println!();
                println!("Configuration:");
                println!("  --set-api-key KEY       Store Helius API key in config");
                println!("  --clear-api-key         Remove API key");
                println!("  --set-network NET       Set default network (mainnet|devnet)");
                println!("  --migrate-config-to-bitwarden [--force]  Copy local config to Bitwarden");
                println!("  --config-path           Show active config location");
                println!("  --status                Show full status");
                return Ok(true);
            }
            _ => {}
        }
    }

    Ok(false)
}

fn store_secret_for_wallet(wallet_id: &str, secret: &str) -> Result<(), Box<dyn Error>> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, wallet_id)?;
    entry.set_password(secret)?;
    Ok(())
}

fn load_secret_for_wallet(wallet_id: &str) -> Result<String, Box<dyn Error>> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, wallet_id)?;
    Ok(entry.get_password()?)
}

fn clear_secret_for_wallet(wallet_id: &str) -> Result<(), Box<dyn Error>> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, wallet_id)?;
    match entry.delete_password() {
        Ok(_) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn keychain_status_summary(config: &DenConfig) -> String {
    let with_keys = config.wallets.iter().filter(|w| w.has_key).count();
    let watch_only = config.wallets.iter().filter(|w| !w.has_key).count();
    if with_keys == 0 && watch_only == 0 {
        "Keychain: no wallets".to_string()
    } else {
        format!("Keychain: {} keys, {} watch-only", with_keys, watch_only)
    }
}

fn load_api_key() -> Result<String, Box<dyn Error>> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_API_KEY_ACCOUNT)?;
    Ok(entry.get_password()?)
}

fn clear_api_key() -> Result<(), Box<dyn Error>> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_API_KEY_ACCOUNT)?;
    match entry.delete_password() {
        Ok(_) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn api_key_status(config: &DenConfig) -> String {
    if std::env::var("HELIUS_API_KEY").is_ok() {
        return "API Key: set (env)".to_string();
    }
    if config.network.api_key.is_some() {
        return "API Key: set (config)".to_string();
    }
    "API Key: not set -- run: den --set-api-key <key>".to_string()
}

fn keypair_from_secret(secret: &str) -> Result<Keypair, Box<dyn Error>> {
    let trimmed = secret.trim();

    if trimmed.starts_with('[') {
        let bytes: Vec<u8> = serde_json::from_str(trimmed)?;
        return keypair_from_bytes(&bytes);
    }

    let bytes = bs58::decode(trimmed).into_vec()?;
    keypair_from_bytes(&bytes)
}

fn keypair_from_bytes(bytes: &[u8]) -> Result<Keypair, Box<dyn Error>> {
    match bytes.len() {
        64 => Ok(Keypair::try_from(bytes)?),
        32 => {
            let seed: [u8; 32] = bytes.try_into()?;
            Ok(Keypair::new_from_array(seed))
        }
        _ => Err("Secret must be 32 or 64 bytes".into()),
    }
}

fn sign_message_with_wallet(wallet_id: &str, message: &str) -> Result<String, Box<dyn Error>> {
    let secret = load_secret_for_wallet(wallet_id)?;
    let keypair = keypair_from_secret(&secret)?;
    let signature = keypair.sign_message(message.as_bytes());
    Ok(signature.to_string())
}

fn resolve_api_key(config: &DenConfig) -> Option<String> {
    std::env::var("HELIUS_API_KEY")
        .ok()
        .or_else(|| config.network.api_key.clone())
}

fn build_rpc_url(api_key: &str, network: Network) -> String {
    match network {
        Network::Mainnet => format!("https://rpc.helius.xyz/?api-key={}", api_key),
        Network::Devnet => format!("https://rpc-devnet.helius.xyz/?api-key={}", api_key),
    }
}

fn fetch_sol_balance(
    client: &reqwest::blocking::Client,
    rpc_url: &str,
    address: &str,
) -> Result<f64, Box<dyn Error>> {
    let result = rpc_call(client, rpc_url, "getBalance", json!([address]))?;
    let lamports = result
        .get("value")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    Ok(lamports as f64 / 1_000_000_000.0)
}

fn refresh_wallet_data(app: &mut App) {
    let den_config = load_den_config();
    app.keystore_status = keychain_status_summary(&den_config);
    app.api_key_status = api_key_status(&den_config);

    let api_key = match resolve_api_key(&den_config) {
        Some(key) => key,
        None => {
            app.status = "No API key. Run: den --set-api-key <key>".to_string();
            // Still populate accounts from config without balances
            app.accounts = den_config
                .wallets
                .iter()
                .map(|w| Account {
                    id: w.id.clone(),
                    name: w.name.clone(),
                    address: w.address.clone(),
                    balance: "-.-- SOL".to_string(),
                    has_key: w.has_key,
                    is_active: den_config.active_wallet.as_deref() == Some(w.id.as_str()),
                    added_at: w.added_at.clone(),
                })
                .collect();
            return;
        }
    };

    let rpc_url = build_rpc_url(&api_key, app.network);
    let client = reqwest::blocking::Client::new();

    // Build accounts list, fetching SOL balance for each
    let mut accounts: Vec<Account> = Vec::new();
    for wallet in &den_config.wallets {
        let is_active = den_config.active_wallet.as_deref() == Some(wallet.id.as_str());
        let balance = fetch_sol_balance(&client, &rpc_url, &wallet.address)
            .map(|b| format!("{:.4} SOL", b))
            .unwrap_or_else(|_| "?.?? SOL".to_string());

        accounts.push(Account {
            id: wallet.id.clone(),
            name: wallet.name.clone(),
            address: wallet.address.clone(),
            balance,
            has_key: wallet.has_key,
            is_active,
            added_at: wallet.added_at.clone(),
        });
    }
    app.accounts = accounts;

    // Full fetch for active wallet only
    if let Some(active) = active_wallet(&den_config) {
        let config = Config {
            address: active.address.clone(),
            rpc_url,
        };
        app.active_wallet_id = Some(active.id.clone());
        app.wallet_address = short_address(&active.address);

        match fetch_wallet_data(&config) {
            Ok(data) => {
                app.total_balance = format!("{:.4} SOL", data.sol_balance);
                // Update active account balance with precise DAS value
                if let Some(acc) = app.accounts.iter_mut().find(|a| a.is_active) {
                    acc.balance = app.total_balance.clone();
                }
                app.apply_active_data(data);
            }
            Err(err) => app.status = format!("Helius error: {}", err),
        }
    } else if den_config.wallets.is_empty() {
        app.status = "No wallets. Press 'a' on Accounts tab to add one".to_string();
        app.wallet_address = "Unset".to_string();
    } else {
        app.status = "No active wallet selected".to_string();
    }
}

fn fetch_wallet_data(config: &Config) -> Result<WalletData, Box<dyn Error>> {
    let client = reqwest::blocking::Client::new();

    let das_result = das_get_assets(&client, &config.rpc_url, &config.address)?;

    let history = rpc_get_history(&client, &config.rpc_url, &config.address)?;

    Ok(WalletData {
        sol_balance: das_result.sol_balance,
        tokens: das_result.tokens,
        history,
    })
}

struct DasResult {
    sol_balance: f64,
    tokens: Vec<Token>,
}

fn das_get_assets(
    client: &reqwest::blocking::Client,
    url: &str,
    address: &str,
) -> Result<DasResult, Box<dyn Error>> {
    let params = json!({
        "ownerAddress": address,
        "page": 1,
        "limit": 1000,
        "displayOptions": {
            "showFungible": true,
            "showNativeBalance": true
        }
    });

    let result = rpc_call(client, url, "getAssetsByOwner", params)?;

    // Native SOL balance
    let sol_balance = result
        .get("nativeBalance")
        .and_then(|nb| nb.get("lamports"))
        .and_then(|l| l.as_u64())
        .map(|l| l as f64 / 1_000_000_000.0)
        .unwrap_or(0.0);

    let sol_price = result
        .get("nativeBalance")
        .and_then(|nb| nb.get("price_per_sol"))
        .and_then(|p| p.as_f64());

    let sol_value = match sol_price {
        Some(price) => format!("${:.2}", sol_balance * price),
        None => "-".to_string(),
    };

    let mut tokens = vec![Token {
        symbol: "SOL".to_string(),
        balance: format!("{:.4}", sol_balance),
        value: sol_value,
        history: seeded_series("SOL", 16),
    }];

    // Fungible tokens from DAS
    if let Some(items) = result.get("items").and_then(|i| i.as_array()) {
        for item in items {
            let interface = item
                .get("interface")
                .and_then(|i| i.as_str())
                .unwrap_or("");

            // Skip non-fungible assets
            if interface != "FungibleToken" && interface != "FungibleAsset" {
                continue;
            }

            let token_info = match item.get("token_info") {
                Some(ti) => ti,
                None => continue,
            };

            let symbol = item
                .get("content")
                .and_then(|c| c.get("metadata"))
                .and_then(|m| m.get("symbol"))
                .and_then(|s| s.as_str())
                .unwrap_or_else(|| {
                    item.get("id")
                        .and_then(|id| id.as_str())
                        .unwrap_or("???")
                });

            let decimals = token_info
                .get("decimals")
                .and_then(|d| d.as_u64())
                .unwrap_or(0);

            let raw_balance = token_info
                .get("balance")
                .and_then(|b| b.as_u64())
                .unwrap_or(0);

            let ui_balance = raw_balance as f64 / 10f64.powi(decimals as i32);

            let price_per_token = token_info
                .get("price_info")
                .and_then(|pi| pi.get("price_per_token"))
                .and_then(|p| p.as_f64());

            let value = match price_per_token {
                Some(price) => format!("${:.2}", ui_balance * price),
                None => "-".to_string(),
            };

            let display_symbol = symbol.to_string();
            tokens.push(Token {
                symbol: display_symbol.clone(),
                balance: format_token_balance(ui_balance, decimals),
                value,
                history: seeded_series(&display_symbol, 16),
            });
        }
    }

    Ok(DasResult { sol_balance, tokens })
}

fn format_token_balance(balance: f64, decimals: u64) -> String {
    if balance == 0.0 {
        return "0".to_string();
    }
    let precision = match decimals {
        0 => 0,
        1..=4 => decimals as usize,
        _ => 4,
    };
    format!("{:.prec$}", balance, prec = precision)
}

fn rpc_get_history(
    client: &reqwest::blocking::Client,
    url: &str,
    address: &str,
) -> Result<Vec<Transaction>, Box<dyn Error>> {
    let params = json!([address, { "limit": 10 }]);
    let result = rpc_call(client, url, "getSignaturesForAddress", params)?;
    let mut history = Vec::new();

    if let Some(values) = result.as_array() {
        for item in values {
            let signature = item
                .get("signature")
                .and_then(|signature| signature.as_str())
                .unwrap_or("Unknown");
            let slot = item.get("slot").and_then(|slot| slot.as_u64()).unwrap_or(0);
            let failed = item.get("err").and_then(|err| err.as_object()).is_some();

            history.push(Transaction {
                time: format!("slot {}", slot),
                summary: if failed {
                    "Failed tx".to_string()
                } else {
                    format!("Tx {}", short_address(signature))
                },
                amount: "-".to_string(),
            });
        }
    }

    Ok(history)
}

fn rpc_call(
    client: &reqwest::blocking::Client,
    url: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });

    let response = client.post(url).json(&body).send()?.error_for_status()?;
    let value: serde_json::Value = response.json()?;

    if let Some(error) = value.get("error") {
        return Err(format!("rpc error: {}", error).into());
    }

    Ok(value.get("result").cloned().unwrap_or_default())
}

fn short_address(value: &str) -> String {
    let length = value.len();
    if length <= 8 {
        return value.to_string();
    }
    format!("{}...{}", &value[..4], &value[length - 4..])
}

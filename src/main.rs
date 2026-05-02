use std::error::Error;
use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::sync::{Mutex, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, SystemTime};

use base64::{Engine as _, engine::general_purpose};
use bip39::{Language, Mnemonic};
use chrono::Utc;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use qrcode::QrCode;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::prelude::{Backend, CrosstermBackend, Terminal};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Row, Table, Tabs, Wrap};
use serde::{Deserialize, Serialize};
use serde_json::json;
use solana_derivation_path::DerivationPath;
use solana_keypair::seed_derivable::keypair_from_seed_and_derivation_path;
use solana_sdk::hash::Hash;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::transaction::Transaction as SolanaTransaction;
use solana_system_interface::instruction as system_instruction;
use spl_associated_token_account::get_associated_token_address;
use spl_associated_token_account::instruction::create_associated_token_account_idempotent;

const KEYCHAIN_SERVICE: &str = "den-wallet";
const KEYCHAIN_API_KEY_ACCOUNT: &str = "helius-api-key";
const CONFIG_DIR_NAME: &str = "den";
const CONFIG_FILE_NAME: &str = "config.toml";
const CONFIG_CACHE_FILE_NAME: &str = "config-cache.json";
const BOOTSTRAP_FILE_NAME: &str = "bootstrap.json";
const CONTACTS_FILE_NAME: &str = "contacts.json";
const CONFIG_BACKEND_ENV: &str = "DEN_CONFIG_BACKEND";
const BW_CONFIG_ITEM_ID_ENV: &str = "DEN_BW_CONFIG_ITEM_ID";
const RAW_KEY_ORIGIN: &str = "raw";
const MNEMONIC_KEY_ORIGIN: &str = "mnemonic";
const DEFAULT_DERIVATION_PATH: &str = "m/44'/501'/0'/0'";
const COMPACT_WIDTH: u16 = 60;
const MEDIUM_WIDTH: u16 = 90;
const QR_MIN_WIDTH: u16 = 58;

static CONFIG_REV: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static BW_SESSION_CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();

mod theme;
use theme::{init_den_theme, reload_den_theme_if_changed, theme};
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

    fn short_title(self) -> &'static str {
        match self {
            Tab::Overview => "Ov",
            Tab::Accounts => "Acct",
            Tab::Tokens => "Tok",
            Tab::Send => "Send",
            Tab::Receive => "Recv",
            Tab::History => "Hist",
            Tab::AddressBook => "Addr",
            Tab::Settings => "Set",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }
}

#[derive(Clone, Debug)]
struct Token {
    symbol: String,
    balance: String,
    value: String,
    mint: Option<String>,
    decimals: u8,
    token_program: Option<String>,
}

#[derive(Clone, Debug)]
struct Account {
    id: String,
    name: String,
    address: String,
    balance: String,
    has_key: bool,
    is_active: bool,
    added_at: Option<String>,
}

#[derive(Clone, Debug)]
struct Nft {
    name: String,
    collection: String,
    address: String,
}

#[derive(Clone, Debug)]
struct Transaction {
    time: String,
    summary: String,
    amount: String,
    signature: String,
    slot: u64,
    failed: bool,
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

#[derive(Clone, Debug)]
struct SendReview {
    from_wallet_id: String,
    from_name: String,
    from_address: String,
    to_address: String,
    asset_symbol: String,
    amount_display: String,
    raw_amount: u64,
    token_mint: Option<String>,
    token_decimals: u8,
    creates_recipient_ata: bool,
    fee_estimate: String,
    simulation_units: Option<u64>,
    network: Network,
}

fn default_contact_network() -> String {
    "mainnet".to_string()
}

fn default_contacts_version() -> u32 {
    1
}

#[derive(Clone, Debug)]
struct WalletData {
    sol_balance: f64,
    tokens: Vec<Token>,
    nfts: Vec<Nft>,
    history: Vec<Transaction>,
}

struct Config {
    address: String,
    rpc_url: String,
    supports_das: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Network {
    Mainnet,
    Devnet,
    Custom,
}

impl Network {
    fn toggle(self) -> Self {
        match self {
            Network::Mainnet => Network::Devnet,
            Network::Devnet => Network::Custom,
            Network::Custom => Network::Mainnet,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Network::Mainnet => "Mainnet",
            Network::Devnet => "Devnet",
            Network::Custom => "Custom",
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
    #[serde(default = "default_key_origin")]
    key_origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    derivation_path: Option<String>,
    #[serde(default)]
    added_at: Option<String>,
}

fn default_key_origin() -> String {
    RAW_KEY_ORIGIN.to_string()
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    custom_rpc_url: Option<String>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            default: default_network(),
            api_key: None,
            custom_rpc_url: None,
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
    bootstrap
        .bitwarden_item_id
        .filter(|id| !id.trim().is_empty())
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
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

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
        &[
            ("BW_CLIENTID", client_id),
            ("BW_CLIENTSECRET", client_secret),
        ],
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

fn persist_backend_choice(
    backend: ConfigBackend,
    bitwarden_item_id: Option<String>,
) -> Result<(), Box<dyn Error>> {
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
    let notes = item.get("notes").and_then(|n| n.as_str()).unwrap_or("");

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
        key_origin: if has_key {
            RAW_KEY_ORIGIN.to_string()
        } else {
            "watch".to_string()
        },
        derivation_path: None,
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

fn validate_solana_address(address: &str) -> Result<(), String> {
    let trimmed = address.trim();
    if trimmed.is_empty() {
        return Err("Address cannot be empty".to_string());
    }
    trimmed
        .parse::<Pubkey>()
        .map(|_| ())
        .map_err(|_| "Address must be a valid Solana public key".to_string())
}

fn contact_address_exists(
    contacts: &[Contact],
    address: &str,
    except_index: Option<usize>,
) -> bool {
    contacts
        .iter()
        .enumerate()
        .any(|(idx, contact)| Some(idx) != except_index && contact.address.trim() == address.trim())
}

fn persist_contacts(contacts: &[Contact]) -> Result<(), Box<dyn Error>> {
    let mut file = load_contacts();
    file.contacts = contacts.to_vec();
    save_contacts(&file)
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
    SendRecipient,
    SendAmount,
    ConfirmSend,
    GenerateWalletName,
    GenerateMnemonicName,
    RestoreMnemonicName,
    RestoreMnemonicPhrase,
    ConfirmMnemonicSaved,
    RevealSecretConfirm,
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

struct PendingMnemonicWallet {
    name: String,
    mnemonic: String,
    secret: String,
    address: String,
    derivation_path: String,
}

struct RevealedSecret {
    label: String,
    value: String,
    kind: String,
}

#[derive(Clone, Debug)]
struct RefreshSnapshot {
    accounts: Vec<Account>,
    active_wallet_id: Option<String>,
    wallet_address: String,
    total_balance: String,
    tokens: Vec<Token>,
    nfts: Vec<Nft>,
    history: Vec<Transaction>,
    keystore_status: String,
    api_key_status: String,
    status: String,
}

type RefreshMessage = Result<RefreshSnapshot, String>;

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
    nfts: Vec<Nft>,
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
    history_detail_index: Option<usize>,
    last_signature: String,
    send_recipient: String,
    pending_send: Option<SendReview>,
    pending_mnemonic: Option<PendingMnemonicWallet>,
    revealed_secret: Option<RevealedSecret>,
    refresh_tx: mpsc::Sender<RefreshMessage>,
    refresh_rx: mpsc::Receiver<RefreshMessage>,
    refresh_in_flight: bool,
    refresh_tick: usize,
    onboarding: OnboardingState,
    theme_mtime: Option<SystemTime>,
}

impl App {
    fn new_placeholder() -> Self {
        let (refresh_tx, refresh_rx) = mpsc::channel();
        Self {
            should_quit: false,
            tab: Tab::Overview,
            accounts: Vec::new(),
            tokens: vec![placeholder_sol_token()],
            history: vec![placeholder_transaction()],
            contacts: Vec::new(),
            selected_account: 0,
            selected_token: 0,
            selected_history: 0,
            selected_contact: 0,
            total_balance: "0.00 SOL".to_string(),
            wallet_address: "Unset".to_string(),
            active_wallet_id: None,
            nfts: Vec::new(),
            status: "Add a wallet: press 'a' on Accounts tab or run: den --add-wallet <name>"
                .to_string(),
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
            history_detail_index: None,
            last_signature: "-".to_string(),
            send_recipient: String::new(),
            pending_send: None,
            pending_mnemonic: None,
            revealed_secret: None,
            refresh_tx,
            refresh_rx,
            refresh_in_flight: false,
            refresh_tick: 0,
            onboarding: OnboardingState {
                active: false,
                step: OnboardingStep::ChooseBackend,
                input: String::new(),
                message: String::new(),
                bw_client_id: String::new(),
            },
            theme_mtime: None,
        }
    }

    fn apply_refresh_snapshot(&mut self, snapshot: RefreshSnapshot) {
        self.accounts = snapshot.accounts;
        self.active_wallet_id = snapshot.active_wallet_id;
        self.wallet_address = snapshot.wallet_address;
        self.total_balance = snapshot.total_balance;
        self.tokens = snapshot.tokens;
        self.nfts = snapshot.nfts;
        self.history = snapshot.history;
        self.keystore_status = snapshot.keystore_status;
        self.api_key_status = snapshot.api_key_status;
        self.status = snapshot.status;
        self.refresh_in_flight = false;
    }

    fn start_refresh(&mut self) {
        if self.refresh_in_flight {
            self.status = "Refresh already running".to_string();
            return;
        }

        self.refresh_in_flight = true;
        self.refresh_tick = 0;
        self.status = "Refreshing wallet data...".to_string();
        let tx = self.refresh_tx.clone();
        let network = self.network;
        thread::spawn(move || {
            let result = build_refresh_snapshot(network).map_err(|err| err.to_string());
            let _ = tx.send(result);
        });
    }

    fn drain_refresh_results(&mut self) {
        loop {
            match self.refresh_rx.try_recv() {
                Ok(Ok(snapshot)) => self.apply_refresh_snapshot(snapshot),
                Ok(Err(err)) => {
                    self.refresh_in_flight = false;
                    self.status = format!("Refresh failed: {}", err);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.refresh_in_flight = false;
                    self.status = "Refresh worker disconnected".to_string();
                    break;
                }
            }
        }
    }

    fn refresh_status_label(&self) -> String {
        if !self.refresh_in_flight {
            return "Idle".to_string();
        }
        let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        format!("{} Loading", spinner[self.refresh_tick % spinner.len()])
    }

    fn on_tick(&mut self) {
        if self.refresh_in_flight {
            self.refresh_tick = self.refresh_tick.wrapping_add(1);
        }
        self.drain_refresh_results();
    }

    fn copy_context_to_clipboard(&mut self) {
        if let Some(secret) = &self.revealed_secret {
            match copy_to_clipboard(&secret.value) {
                Ok(_) => self.status = format!("Copied {} to clipboard", secret.kind),
                Err(err) => self.status = format!("Clipboard copy failed: {}", err),
            }
            return;
        }

        let target = match self.tab {
            Tab::Accounts => self
                .accounts
                .get(self.selected_account)
                .map(|account| ("wallet address", account.address.clone())),
            Tab::Receive | Tab::Overview | Tab::Settings => self
                .accounts
                .iter()
                .find(|account| account.is_active)
                .map(|account| ("active wallet address", account.address.clone())),
            Tab::AddressBook => {
                let idx = self.contact_detail_index.unwrap_or(self.selected_contact);
                self.contacts
                    .get(idx)
                    .map(|contact| ("contact address", contact.address.clone()))
            }
            Tab::History => {
                let idx = self.history_detail_index.unwrap_or(self.selected_history);
                self.history.get(idx).and_then(|tx| {
                    if tx.signature.is_empty() {
                        None
                    } else {
                        Some(("transaction signature", tx.signature.clone()))
                    }
                })
            }
            _ => None,
        };

        let Some((label, value)) = target else {
            self.status = "Nothing to copy in this view".to_string();
            return;
        };

        match copy_to_clipboard(&value) {
            Ok(_) => self.status = format!("Copied {} to clipboard", label),
            Err(err) => self.status = format!("Clipboard copy failed: {}", err),
        }
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

        if self.revealed_secret.is_some() {
            match code {
                KeyCode::Esc => self.revealed_secret = None,
                KeyCode::Char('c') => self.copy_context_to_clipboard(),
                KeyCode::Char('q') => self.should_quit = true,
                _ => {}
            }
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
                self.start_refresh();
                self.status = msg;
            }
            KeyCode::Char('r') => {
                self.start_refresh();
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
            KeyCode::Char('g') => {
                if self.tab == Tab::Accounts {
                    self.input_mode = InputMode::GenerateWalletName;
                    self.input_buffer.clear();
                    self.import_state.wallet_name.clear();
                }
            }
            KeyCode::Char('m') => {
                if self.tab == Tab::Accounts {
                    self.input_mode = InputMode::GenerateMnemonicName;
                    self.input_buffer.clear();
                    self.import_state.wallet_name.clear();
                }
            }
            KeyCode::Char('p') => {
                if self.tab == Tab::Accounts {
                    self.input_mode = InputMode::RestoreMnemonicName;
                    self.input_buffer.clear();
                    self.import_state.wallet_name.clear();
                }
            }
            KeyCode::Char('x') => {
                if self.tab == Tab::Accounts && !self.accounts.is_empty() {
                    self.input_mode = InputMode::RevealSecretConfirm;
                    self.input_buffer.clear();
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
                        self.start_refresh();
                        self.status = msg;
                    } else {
                        self.wallet_detail_index = Some(self.selected_account);
                    }
                } else if self.tab == Tab::Send {
                    if self.pending_send.is_some() {
                        self.input_mode = InputMode::ConfirmSend;
                        self.input_buffer.clear();
                    } else {
                        self.start_send_flow();
                    }
                } else if self.tab == Tab::History
                    && self.history_detail_index.is_none()
                    && !self.history.is_empty()
                {
                    self.history_detail_index = Some(self.selected_history);
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
                } else if self.tab == Tab::History && self.history_detail_index.is_some() {
                    self.history_detail_index = None;
                } else if self.tab == Tab::Send && self.pending_send.is_some() {
                    self.pending_send = None;
                    self.status = "Send cancelled".to_string();
                }
            }
            KeyCode::Char('c') => {
                self.copy_context_to_clipboard();
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

    fn selected_send_token(&self) -> Token {
        self.tokens
            .get(self.selected_token)
            .cloned()
            .or_else(|| self.tokens.first().cloned())
            .unwrap_or_else(placeholder_sol_token)
    }

    fn start_send_flow(&mut self) {
        let config = load_den_config();
        match active_wallet(&config) {
            Some(wallet) if !wallet.has_key => {
                self.status = format!("Cannot send: '{}' is watch-only", wallet.name);
            }
            Some(_) => {
                self.pending_send = None;
                self.send_recipient.clear();
                self.input_mode = InputMode::SendRecipient;
                self.input_buffer.clear();
            }
            None => {
                self.status = "No active wallet".to_string();
            }
        }
    }

    fn prepare_send_review(&mut self, amount: &str) {
        let config = load_den_config();
        let Some(wallet) = active_wallet(&config).cloned() else {
            self.status = "No active wallet".to_string();
            return;
        };
        if !wallet.has_key {
            self.status = format!("Cannot send: '{}' is watch-only", wallet.name);
            return;
        }
        let rpc_url = match rpc_url_for_network(&config, self.network) {
            Ok(url) => url,
            Err(err) => {
                self.status = format!("Cannot send: {}", err);
                return;
            }
        };
        let token = self.selected_send_token();
        match build_send_review(
            &wallet,
            &token,
            &self.send_recipient,
            amount,
            &rpc_url,
            self.network,
        ) {
            Ok(review) => {
                self.pending_send = Some(review);
                self.status =
                    "Simulation passed. Review details, then press Enter to confirm.".to_string();
            }
            Err(err) => {
                self.pending_send = None;
                self.status = format!("Send blocked: {}", err);
            }
        }
    }

    fn reveal_selected_wallet_secret(&mut self) -> Result<(), Box<dyn Error>> {
        let Some(account) = self.accounts.get(self.selected_account) else {
            return Err("no selected wallet".into());
        };
        if !account.has_key {
            return Err("watch-only wallets do not have secrets".into());
        }

        let config = load_den_config();
        let wallet = config
            .wallets
            .iter()
            .find(|wallet| wallet.id == account.id)
            .ok_or("wallet not found")?;

        if wallet.key_origin == MNEMONIC_KEY_ORIGIN {
            let mnemonic = load_mnemonic_for_wallet(&wallet.id)?;
            self.revealed_secret = Some(RevealedSecret {
                label: wallet.name.clone(),
                value: mnemonic,
                kind: "seed phrase".to_string(),
            });
        } else {
            let secret = load_secret_for_wallet(&wallet.id)?;
            let keypair = keypair_from_secret(&secret)?;
            self.revealed_secret = Some(RevealedSecret {
                label: wallet.name.clone(),
                value: keypair_to_base58_secret(&keypair),
                kind: "private key".to_string(),
            });
        }
        Ok(())
    }

    fn confirm_pending_send(&mut self, input: &str) {
        if input != "SEND" {
            self.status = "Send cancelled".to_string();
            return;
        }
        let Some(review) = self.pending_send.clone() else {
            self.status = "No pending send".to_string();
            return;
        };
        let config = load_den_config();
        let rpc_url = match rpc_url_for_network(&config, review.network) {
            Ok(url) => url,
            Err(err) => {
                self.status = format!("Send failed: {}", err);
                return;
            }
        };
        match broadcast_send(&review, &rpc_url) {
            Ok(signature) => {
                self.last_signature = signature.clone();
                self.pending_send = None;
                self.status = format!("Transaction sent: {}", short_address(&signature));
                self.start_refresh();
            }
            Err(err) => {
                self.status = format!("Send failed: {}", err);
            }
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
        self.start_refresh();
        self.status = status.to_string();
    }

    fn handle_onboarding_mode(&mut self, code: KeyCode) {
        match self.onboarding.step {
            OnboardingStep::ChooseBackend => match code {
                KeyCode::Char('1') => match persist_backend_choice(ConfigBackend::Local, None) {
                    Ok(_) => self.complete_onboarding("Setup complete: using local config"),
                    Err(err) => {
                        self.onboarding.message = format!("Setup failed: {}", err);
                    }
                },
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
                KeyCode::Char('i') => match bw_status() {
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
                },
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
                                    "Bitwarden login successful. Press u to unlock vault."
                                        .to_string();
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

                    match initialize_bitwarden_config_item(&item_id).and_then(|_| {
                        persist_backend_choice(ConfigBackend::Bitwarden, Some(item_id.clone()))
                    }) {
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
                if self.input_mode == InputMode::ConfirmMnemonicSaved {
                    self.pending_mnemonic = None;
                    self.status = "Mnemonic wallet cancelled; no key was stored".to_string();
                }
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
                                                key_origin: RAW_KEY_ORIGIN.to_string(),
                                                derivation_path: None,
                                                added_at: Some(
                                                    Utc::now().format("%Y-%m-%d").to_string(),
                                                ),
                                            });
                                            if config.active_wallet.is_none() {
                                                config.active_wallet = Some(wallet_id);
                                            }
                                            let _ = save_den_config(&config);
                                            let msg = format!("Wallet '{}' imported", name);
                                            self.start_refresh();
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
                                key_origin: "watch".to_string(),
                                derivation_path: None,
                                added_at: Some(Utc::now().format("%Y-%m-%d").to_string()),
                            });
                            if config.active_wallet.is_none() {
                                config.active_wallet = Some(wallet_id);
                            }
                            let _ = save_den_config(&config);
                            let msg = format!("Watch-only wallet '{}' added", name);
                            self.start_refresh();
                            self.status = msg;
                        }
                    }
                    InputMode::GenerateWalletName => {
                        if input.is_empty() {
                            self.status = "Generate cancelled".to_string();
                        } else {
                            match create_random_wallet(&input) {
                                Ok(address) => {
                                    self.start_refresh();
                                    self.status = format!(
                                        "Generated wallet '{}' ({})",
                                        input,
                                        short_address(&address)
                                    );
                                }
                                Err(err) => self.status = format!("Generate failed: {}", err),
                            }
                        }
                    }
                    InputMode::GenerateMnemonicName => {
                        if input.is_empty() {
                            self.status = "Mnemonic wallet cancelled".to_string();
                        } else {
                            match prepare_mnemonic_wallet(&input) {
                                Ok(pending) => {
                                    self.pending_mnemonic = Some(pending);
                                    self.input_mode = InputMode::ConfirmMnemonicSaved;
                                    self.input_buffer.clear();
                                    self.status =
                                        "Back up the seed phrase, then type I SAVED IT".to_string();
                                    return;
                                }
                                Err(err) => self.status = format!("Mnemonic failed: {}", err),
                            }
                        }
                    }
                    InputMode::RestoreMnemonicName => {
                        if input.is_empty() {
                            self.status = "Mnemonic restore cancelled".to_string();
                        } else {
                            self.import_state.wallet_name = input;
                            self.input_mode = InputMode::RestoreMnemonicPhrase;
                            self.input_buffer.clear();
                            return;
                        }
                    }
                    InputMode::RestoreMnemonicPhrase => {
                        if input.is_empty() {
                            self.status = "Mnemonic restore cancelled".to_string();
                        } else {
                            match restore_mnemonic_wallet(&self.import_state.wallet_name, &input) {
                                Ok(address) => {
                                    self.start_refresh();
                                    self.status = format!(
                                        "Mnemonic wallet restored ({})",
                                        short_address(&address)
                                    );
                                }
                                Err(err) => {
                                    self.status = format!("Mnemonic restore failed: {}", err)
                                }
                            }
                        }
                    }
                    InputMode::ConfirmMnemonicSaved => {
                        if input != "I SAVED IT" {
                            self.pending_mnemonic = None;
                            self.status =
                                "Mnemonic wallet cancelled; no key was stored".to_string();
                        } else if let Some(pending) = self.pending_mnemonic.take() {
                            match store_mnemonic_wallet(pending) {
                                Ok(address) => {
                                    self.start_refresh();
                                    self.status = format!(
                                        "Mnemonic wallet stored ({})",
                                        short_address(&address)
                                    );
                                }
                                Err(err) => self.status = format!("Mnemonic store failed: {}", err),
                            }
                        } else {
                            self.status = "No pending mnemonic wallet".to_string();
                        }
                    }
                    InputMode::RevealSecretConfirm => {
                        if input != "REVEAL" {
                            self.status = "Secret reveal cancelled".to_string();
                        } else {
                            match self.reveal_selected_wallet_secret() {
                                Ok(_) => {
                                    self.status = "Secret revealed; press c to copy or Esc to close"
                                        .to_string()
                                }
                                Err(err) => self.status = format!("Reveal failed: {}", err),
                            }
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
                                self.start_refresh();
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
                                config.active_wallet = config.wallets.first().map(|w| w.id.clone());
                            }
                            if had_key {
                                let _ = clear_secret_for_wallet(&wallet_id);
                                let _ = clear_mnemonic_for_wallet(&wallet_id);
                            }
                            let _ = save_den_config(&config);
                            self.selected_account = 0;
                            self.wallet_detail_index = None;
                            let msg = format!("Wallet '{}' removed", wallet_name);
                            self.start_refresh();
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
                        } else if let Err(err) = validate_solana_address(&input) {
                            self.status = err;
                            return;
                        } else if contact_address_exists(&self.contacts, &input, None) {
                            self.status = "Contact address already exists".to_string();
                            return;
                        } else {
                            let contact = Contact {
                                name: self.import_state.wallet_name.clone(),
                                address: input,
                                network: self.network.label().to_ascii_lowercase(),
                                notes: String::new(),
                            };
                            let name = contact.name.clone();
                            self.contacts.push(contact);
                            match persist_contacts(&self.contacts) {
                                Ok(_) => self.status = format!("Contact '{}' added", name),
                                Err(err) => self.status = format!("Contact save failed: {}", err),
                            }
                        }
                    }
                    InputMode::EditContactName => {
                        if input.is_empty() {
                            self.status = "Edit cancelled".to_string();
                        } else {
                            let idx = self.contact_detail_index.unwrap_or(self.selected_contact);
                            if idx < self.contacts.len() {
                                self.contacts[idx].name = input.clone();
                                match persist_contacts(&self.contacts) {
                                    Ok(_) => {
                                        self.status = format!("Contact updated to '{}'", input)
                                    }
                                    Err(err) => {
                                        self.status = format!("Contact save failed: {}", err)
                                    }
                                }
                            }
                        }
                    }
                    InputMode::EditContactAddress => {
                        if input.is_empty() {
                            self.status = "Edit cancelled".to_string();
                        } else if let Err(err) = validate_solana_address(&input) {
                            self.status = err;
                            return;
                        } else if let Some(idx) = self.contact_detail_index {
                            if idx < self.contacts.len() {
                                if contact_address_exists(&self.contacts, &input, Some(idx)) {
                                    self.status = "Contact address already exists".to_string();
                                    return;
                                }
                                self.contacts[idx].address = input;
                                match persist_contacts(&self.contacts) {
                                    Ok(_) => self.status = "Address updated".to_string(),
                                    Err(err) => {
                                        self.status = format!("Contact save failed: {}", err)
                                    }
                                }
                            }
                        }
                    }
                    InputMode::EditContactNotes => {
                        if let Some(idx) = self.contact_detail_index {
                            if idx < self.contacts.len() {
                                self.contacts[idx].notes = input;
                                match persist_contacts(&self.contacts) {
                                    Ok(_) => self.status = "Notes updated".to_string(),
                                    Err(err) => {
                                        self.status = format!("Contact save failed: {}", err)
                                    }
                                }
                            }
                        }
                    }
                    InputMode::ConfirmDeleteContact => {
                        if (input == "y" || input == "yes") && !self.contacts.is_empty() {
                            let idx = self.contact_detail_index.unwrap_or(self.selected_contact);
                            if idx < self.contacts.len() {
                                let name = self.contacts[idx].name.clone();
                                self.contacts.remove(idx);
                                match persist_contacts(&self.contacts) {
                                    Ok(_) => {
                                        self.contact_detail_index = None;
                                        if self.selected_contact >= self.contacts.len()
                                            && !self.contacts.is_empty()
                                        {
                                            self.selected_contact = self.contacts.len() - 1;
                                        }
                                        self.status = format!("Contact '{}' deleted", name);
                                    }
                                    Err(err) => {
                                        self.status = format!("Contact save failed: {}", err)
                                    }
                                }
                            }
                        } else {
                            self.status = "Delete cancelled".to_string();
                        }
                    }
                    InputMode::SendRecipient => {
                        if input.is_empty() {
                            self.status = "Send cancelled".to_string();
                        } else if let Err(err) = validate_solana_address(&input) {
                            self.status = err;
                            return;
                        } else {
                            self.send_recipient = input;
                            self.input_mode = InputMode::SendAmount;
                            self.input_buffer.clear();
                            return;
                        }
                    }
                    InputMode::SendAmount => {
                        if input.is_empty() {
                            self.status = "Send cancelled".to_string();
                        } else {
                            self.prepare_send_review(&input);
                        }
                    }
                    InputMode::ConfirmSend => {
                        self.confirm_pending_send(&input);
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
        if self.wallet_detail_index.is_some()
            || self.contact_detail_index.is_some()
            || self.history_detail_index.is_some()
        {
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
            Tab::Tokens | Tab::Send => {
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
        reload_den_theme_if_changed(&mut app.theme_mtime);
        app.on_tick();
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
    init_den_theme();
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
        "custom" => Network::Custom,
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

    app.start_refresh();

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
        render_footer(
            frame,
            layout[2],
            &app.status,
            footer_height,
            app.tab,
            app.wallet_detail_index.is_some()
                || app.contact_detail_index.is_some()
                || app.history_detail_index.is_some()
                || app.pending_send.is_some(),
        );
    }

    if app.onboarding.active {
        render_onboarding_modal(frame, app);
    } else if app.input_mode == InputMode::ConfirmMnemonicSaved {
        render_mnemonic_confirm_modal(frame, app);
    } else if app.input_mode != InputMode::None {
        render_input_modal(frame, app);
    } else if app.revealed_secret.is_some() {
        render_revealed_secret_modal(frame, app);
    }
}

fn render_header(
    frame: &mut ratatui::prelude::Frame,
    area: Rect,
    tab: Tab,
    width: u16,
    network: Network,
) {
    if width < COMPACT_WIDTH {
        let title = format!(
            "Den | {} {} | {}",
            tab.index() + 1,
            tab.short_title(),
            network.label()
        );
        let header = Paragraph::new(title)
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme().border))
                    .title("1-8 nav"),
            )
            .style(Style::default().fg(theme().fg));
        frame.render_widget(header, area);
        return;
    }

    let use_short_titles = width < MEDIUM_WIDTH;
    let titles = Tab::ALL
        .iter()
        .map(|t| {
            let label = if use_short_titles {
                t.short_title()
            } else {
                t.title()
            };
            Line::from(Span::styled(label, Style::default().fg(theme().fg)))
        })
        .collect::<Vec<_>>();

    let tabs = Tabs::new(titles)
        .select(tab.index())
        .highlight_style(
            Style::default()
                .fg(theme().sel_fg)
                .bg(theme().accent)
                .add_modifier(Modifier::BOLD),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme().border))
                .title(format!(
                    "Den Wallet | {} | {}",
                    tab.title(),
                    network.label()
                )),
        )
        .style(Style::default().fg(theme().fg));

    frame.render_widget(tabs, area);
}

fn render_body(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App, width: u16) {
    render_main(frame, area, app, width);
}

fn render_main(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App, width: u16) {
    match app.tab {
        Tab::Overview => render_overview(frame, area, app, width),
        Tab::Accounts => render_accounts(frame, area, app, width),
        Tab::Tokens => render_tokens_view(frame, area, app, width),
        Tab::Send => render_send(frame, area, app, width),
        Tab::Receive => render_receive(frame, area, app, width),
        Tab::History => render_history(frame, area, app),
        Tab::AddressBook => render_address_book(frame, area, app),
        Tab::Settings => render_settings(frame, area, app, width),
    }
}

fn render_overview(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App, width: u16) {
    let compact = width < COMPACT_WIDTH || area.height < 18;
    let summary_height = if compact { 6 } else { 10 };
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(summary_height), Constraint::Min(0)])
        .split(area);

    let overview = if compact {
        Text::from(vec![
            Line::from(format!("Total: {}", app.total_balance)),
            Line::from(format!(
                "Accounts: {} | Tokens: {}",
                app.accounts.len(),
                app.tokens.len()
            )),
            Line::from(format!("Data: {}", app.refresh_status_label())),
            Line::from("1-8 switches sections"),
        ])
    } else {
        let art = [
            "__         __",
            "/  \\.-\"\"\"-.//  \\",
            "\\    -   -    /",
            " |   o   o   |",
            " \\  .-'''-.  /",
        ];

        Text::from(
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
                    Line::from(format!("Data: {}", app.refresh_status_label())),
                ])
                .collect::<Vec<_>>(),
        )
    };

    let paragraph = Paragraph::new(overview)
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme().border))
                .title("Overview"),
        )
        .style(Style::default().fg(theme().fg));

    frame.render_widget(paragraph, layout[0]);

    if width < MEDIUM_WIDTH {
        let bottom = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(layout[1]);
        render_tokens_table(frame, bottom[0], app, width);
        render_history_list(frame, bottom[1], app);
    } else {
        let bottom = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(layout[1]);
        render_tokens_table(frame, bottom[0], app, width);
        render_history_list(frame, bottom[1], app);
    }
}

fn render_accounts(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App, width: u16) {
    if let Some(index) = app.wallet_detail_index {
        render_wallet_detail(frame, area, app, index);
        return;
    }

    let title = if width < COMPACT_WIDTH {
        "Wallets [Enter details | g/m/p/w]"
    } else {
        "Wallets [a:import g:generate m:seed p:restore w:watch x:reveal]"
    };

    if width < COMPACT_WIDTH {
        let items = app
            .accounts
            .iter()
            .map(|account| {
                let marker = if account.is_active { "*" } else { " " };
                let wallet_type = if account.has_key { "Full" } else { "Watch" };
                ListItem::new(Text::from(vec![
                    Line::from(format!("{} {} ({})", marker, account.name, wallet_type)),
                    Line::from(format!(
                        "  {}  {}",
                        short_address(&account.address),
                        account.balance
                    )),
                ]))
            })
            .collect::<Vec<_>>();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme().border))
                    .title(title),
            )
            .highlight_style(
                Style::default()
                    .fg(theme().sel_fg)
                    .bg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ")
            .style(Style::default().fg(theme().fg));

        frame.render_stateful_widget(list, area, &mut list_state(app.selected_account));
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
                .fg(theme().accent)
                .bg(theme().surface)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme().border))
            .title(title),
    )
    .row_highlight_style(
        Style::default()
            .fg(theme().sel_fg)
            .bg(theme().accent)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("> ");

    frame.render_stateful_widget(table, area, &mut table_state(app.selected_account));
}

fn render_wallet_detail(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App, index: usize) {
    let account = match app.accounts.get(index) {
        Some(a) => a,
        None => {
            let msg = Paragraph::new("Wallet not found")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(theme().border))
                        .title("Wallet Detail"),
                )
                .style(Style::default().fg(theme().accent));
            frame.render_widget(msg, area);
            return;
        }
    };

    let wallet_type = if account.has_key {
        "Full (signing key stored)"
    } else {
        "Watch-only"
    };
    let active_status = if account.is_active { "Yes" } else { "No" };
    let added_display = account.added_at.as_deref().unwrap_or("Unknown");
    let config = load_den_config();
    let wallet_config = config.wallets.iter().find(|wallet| wallet.id == account.id);
    let origin_display = wallet_config
        .map(|wallet| wallet.key_origin.as_str())
        .unwrap_or(if account.has_key {
            RAW_KEY_ORIGIN
        } else {
            "watch"
        });
    let derivation_display = wallet_config
        .and_then(|wallet| wallet.derivation_path.as_deref())
        .unwrap_or("-");

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(18), Constraint::Min(0)])
        .split(area);

    let info = Text::from(vec![
        Line::from(vec![
            Span::styled("  Name:     ", Style::default().fg(theme().fg_dim)),
            Span::styled(&account.name, Style::default().fg(theme().fg)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Address:  ", Style::default().fg(theme().fg_dim)),
            Span::styled(&account.address, Style::default().fg(theme().fg)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Balance:  ", Style::default().fg(theme().fg_dim)),
            Span::styled(&account.balance, Style::default().fg(theme().fg)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Type:     ", Style::default().fg(theme().fg_dim)),
            Span::styled(wallet_type, Style::default().fg(theme().fg)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Active:   ", Style::default().fg(theme().fg_dim)),
            Span::styled(
                active_status,
                Style::default().fg(if account.is_active {
                    theme().green
                } else {
                    theme().fg_dim
                }),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Origin:   ", Style::default().fg(theme().fg_dim)),
            Span::styled(origin_display, Style::default().fg(theme().fg)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Path:     ", Style::default().fg(theme().fg_dim)),
            Span::styled(derivation_display, Style::default().fg(theme().fg)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Added:    ", Style::default().fg(theme().fg_dim)),
            Span::styled(added_display, Style::default().fg(theme().fg)),
        ]),
    ]);

    let paragraph = Paragraph::new(info)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme().border))
                .title(format!("Wallet: {}", account.name)),
        )
        .style(Style::default().fg(theme().fg));

    frame.render_widget(paragraph, layout[0]);

    let hints = Text::from(vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  Enter",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  Set as active wallet", Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled(
                "  e",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("      Rename wallet", Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled(
                "  x",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "      Reveal backup secret (requires REVEAL)",
                Style::default().fg(theme().fg),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  d",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("      Delete wallet", Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled(
                "  Esc",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("    Back to wallet list", Style::default().fg(theme().fg)),
        ]),
    ]);

    let actions = Paragraph::new(hints)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme().border))
                .title("Actions"),
        )
        .style(Style::default().fg(theme().fg));

    frame.render_widget(actions, layout[1]);
}

fn render_tokens_view(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App, width: u16) {
    if width < MEDIUM_WIDTH {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);
        render_tokens_table(frame, layout[0], app, width);
        render_token_chart(frame, layout[1], app);
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);
    render_tokens_table(frame, layout[0], app, width);
    render_token_chart(frame, layout[1], app);
}

fn render_tokens_table(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App, width: u16) {
    if width < COMPACT_WIDTH {
        let items = app
            .tokens
            .iter()
            .map(|token| {
                ListItem::new(Text::from(vec![
                    Line::from(format!("{}  {}", token.symbol, token.value)),
                    Line::from(format!(
                        "  {}  {}",
                        token.balance,
                        token_program_label(token)
                    )),
                ]))
            })
            .collect::<Vec<_>>();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme().border))
                    .title("Tokens"),
            )
            .highlight_style(
                Style::default()
                    .fg(theme().sel_fg)
                    .bg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ")
            .style(Style::default().fg(theme().fg));

        frame.render_stateful_widget(list, area, &mut list_state(app.selected_token));
        return;
    }

    let rows = app.tokens.iter().map(|token| {
        Row::new(vec![
            ratatui::widgets::Cell::from(token.symbol.clone()),
            ratatui::widgets::Cell::from(token.balance.clone()),
            ratatui::widgets::Cell::from(token.value.clone()),
            ratatui::widgets::Cell::from(token_program_label(token)),
        ])
    });

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ],
    )
    .header(
        Row::new(vec!["Token", "Balance", "Value", "Program"]).style(
            Style::default()
                .fg(theme().accent)
                .bg(theme().surface)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme().border))
            .title("Tokens"),
    )
    .row_highlight_style(
        Style::default()
            .fg(theme().sel_fg)
            .bg(theme().accent)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("> ");

    frame.render_stateful_widget(table, area, &mut table_state(app.selected_token));
}

fn render_token_chart(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App) {
    let token = app.tokens.get(app.selected_token).or(app.tokens.first());
    let mut lines = Vec::new();

    if let Some(token) = token {
        lines.push(Line::from(vec![
            Span::styled("Selected asset: ", Style::default().fg(theme().fg_dim)),
            Span::styled(&token.symbol, Style::default().fg(theme().fg)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Program: ", Style::default().fg(theme().fg_dim)),
            Span::styled(token_program_label(token), Style::default().fg(theme().fg)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(
        "Charts: unavailable (real historical portfolio data not configured).",
    ));
    lines.push(Line::from("No seeded/fake chart data is shown."));
    lines.push(Line::from(""));
    lines.push(Line::from(format!("NFTs detected: {}", app.nfts.len())));
    for nft in app.nfts.iter().take(5) {
        let collection = if nft.collection == "-" {
            String::new()
        } else {
            format!(" [{}]", short_address(&nft.collection))
        };
        lines.push(Line::from(format!(
            "• {}{} {}",
            nft.name,
            collection,
            short_address(&nft.address)
        )));
    }
    if app.nfts.len() > 5 {
        lines.push(Line::from(format!("…and {} more", app.nfts.len() - 5)));
    }

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme().border))
                    .title("Asset details"),
            )
            .style(Style::default().fg(theme().fg)),
        area,
    );
}

fn render_history(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App) {
    if let Some(index) = app.history_detail_index {
        render_transaction_detail(frame, area, app, index);
    } else {
        render_history_list(frame, area, app);
    }
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
                .border_style(Style::default().fg(theme().border))
                .title("Recent Activity"),
        )
        .highlight_style(
            Style::default()
                .fg(theme().sel_fg)
                .bg(theme().accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ")
        .style(Style::default().fg(theme().fg));

    frame.render_stateful_widget(list, area, &mut list_state(app.selected_history));
}

fn render_transaction_detail(
    frame: &mut ratatui::prelude::Frame,
    area: Rect,
    app: &App,
    index: usize,
) {
    let tx = match app.history.get(index) {
        Some(tx) => tx,
        None => {
            let msg = Paragraph::new("Transaction not found")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(theme().border))
                        .title("Transaction Detail"),
                )
                .style(Style::default().fg(theme().accent));
            frame.render_widget(msg, area);
            return;
        }
    };

    let status = if tx.failed { "Failed" } else { "Confirmed" };
    let signature = if tx.signature.is_empty() {
        "Unavailable"
    } else {
        &tx.signature
    };
    let info = Text::from(vec![
        Line::from(vec![
            Span::styled("  Status:    ", Style::default().fg(theme().fg_dim)),
            Span::styled(
                status,
                Style::default().fg(if tx.failed {
                    theme().red
                } else {
                    theme().green
                }),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Slot:      ", Style::default().fg(theme().fg_dim)),
            Span::styled(tx.slot.to_string(), Style::default().fg(theme().fg)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Summary:   ", Style::default().fg(theme().fg_dim)),
            Span::styled(&tx.summary, Style::default().fg(theme().fg)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Amount:    ", Style::default().fg(theme().fg_dim)),
            Span::styled(&tx.amount, Style::default().fg(theme().fg)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Signature: ", Style::default().fg(theme().fg_dim)),
            Span::styled(signature, Style::default().fg(theme().fg)),
        ]),
        Line::from(""),
        Line::from("  Press c to copy signature, Esc to return."),
    ]);

    let paragraph = Paragraph::new(info)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme().border))
                .title("Transaction Detail"),
        )
        .style(Style::default().fg(theme().fg));
    frame.render_widget(paragraph, area);
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
                .border_style(Style::default().fg(theme().border))
                .title("Address Book [Enter:details a:add e:edit d:delete]"),
        )
        .highlight_style(
            Style::default()
                .fg(theme().sel_fg)
                .bg(theme().accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ")
        .style(Style::default().fg(theme().fg));

    frame.render_stateful_widget(list, area, &mut list_state(app.selected_contact));
}

fn render_contact_detail(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App, index: usize) {
    let contact = match app.contacts.get(index) {
        Some(c) => c,
        None => {
            let msg = Paragraph::new("Contact not found")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(theme().border))
                        .title("Contact Detail"),
                )
                .style(Style::default().fg(theme().accent));
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
            Span::styled("  Name:     ", Style::default().fg(theme().fg_dim)),
            Span::styled(&contact.name, Style::default().fg(theme().fg)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Address:  ", Style::default().fg(theme().fg_dim)),
            Span::styled(&contact.address, Style::default().fg(theme().fg)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Network:  ", Style::default().fg(theme().fg_dim)),
            Span::styled(&contact.network, Style::default().fg(theme().fg)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Notes:    ", Style::default().fg(theme().fg_dim)),
            Span::styled(notes_display, Style::default().fg(theme().fg)),
        ]),
    ]);

    let paragraph = Paragraph::new(info)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme().border))
                .title(format!("Contact: {}", contact.name)),
        )
        .style(Style::default().fg(theme().fg));

    frame.render_widget(paragraph, layout[0]);

    let hints = Text::from(vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  e",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("      Edit name", Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled(
                "  a",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("      Edit address", Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled(
                "  o",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("      Edit notes", Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled(
                "  d",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("      Delete contact", Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled(
                "  Esc",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("    Back to contact list", Style::default().fg(theme().fg)),
        ]),
    ]);

    let actions = Paragraph::new(hints)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme().border))
                .title("Actions"),
        )
        .style(Style::default().fg(theme().fg));

    frame.render_widget(actions, layout[1]);
}

fn render_send(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App, width: u16) {
    if let Some(review) = &app.pending_send {
        render_send_review(frame, area, review);
        return;
    }

    if let Some(acc) = app.accounts.iter().find(|a| a.is_active) {
        if !acc.has_key {
            let notice = Paragraph::new(
                "Watch-only wallet -- signing not available.\nSwitch to a full wallet to send.",
            )
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme().border))
                    .title("Send"),
            )
            .style(Style::default().fg(theme().accent));
            frame.render_widget(notice, area);
            return;
        }
    }

    let (account_name, account_address) = active_account(app);
    let token = app.selected_send_token();
    let asset_hint = if token.mint.is_some() {
        format!(
            "{} ({})",
            token.symbol,
            short_address(token.mint.as_deref().unwrap_or(""))
        )
    } else {
        "SOL".to_string()
    };

    let layout = if width < COMPACT_WIDTH {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(8), Constraint::Min(0)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(8),
                Constraint::Length(7),
                Constraint::Min(0),
            ])
            .split(area)
    };

    let fields = Text::from(vec![
        Line::from(format!("From:   {} ({})", account_name, account_address)),
        Line::from("To:     Enter recipient when prompted"),
        Line::from(format!("Asset:  {}", asset_hint)),
        Line::from(format!("Balance: {}", token.balance)),
        Line::from("Amount: Enter amount when prompted"),
    ]);

    let details = Text::from(vec![
        Line::from(format!("Network: {}", app.network.label())),
        Line::from("Fee: default Solana fee; priority fees deferred"),
        Line::from("Simulation: required; failures block sending"),
        Line::from("SPL Token: recipient ATA is created when missing"),
    ]);

    let actions_text = if width < COMPACT_WIDTH {
        "Up/Down asset | Enter send | Esc cancel"
    } else {
        "Up/Down: choose asset   Enter: enter recipient/amount   Esc: cancel review"
    };
    let actions = Paragraph::new(actions_text)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme().border))
                .title("Actions"),
        )
        .style(Style::default().fg(theme().fg));

    frame.render_widget(
        Paragraph::new(fields)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme().border))
                    .title("Send"),
            )
            .style(Style::default().fg(theme().fg)),
        layout[0],
    );
    if width < COMPACT_WIDTH {
        let compact_details = Text::from(vec![
            Line::from(format!("Network: {}", app.network.label())),
            Line::from("Simulation required; failures block sending"),
            Line::from("Default fee; SPL ATA created when missing"),
            Line::from(""),
            Line::from(actions_text),
        ]);
        frame.render_widget(
            Paragraph::new(compact_details)
                .wrap(Wrap { trim: true })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(theme().border))
                        .title("Details"),
                )
                .style(Style::default().fg(theme().fg)),
            layout[1],
        );
    } else {
        frame.render_widget(
            Paragraph::new(details)
                .wrap(Wrap { trim: true })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(theme().border))
                        .title("Details"),
                )
                .style(Style::default().fg(theme().fg)),
            layout[1],
        );
        frame.render_widget(actions, layout[2]);
    }
}

fn render_send_review(frame: &mut ratatui::prelude::Frame, area: Rect, review: &SendReview) {
    let ata_line = if review.token_mint.is_some() {
        if review.creates_recipient_ata {
            "Recipient token account: will be created"
        } else {
            "Recipient token account: already exists"
        }
    } else {
        "Recipient token account: not needed for SOL"
    };
    let simulation = review
        .simulation_units
        .map(|units| format!("passed ({} compute units)", units))
        .unwrap_or_else(|| "passed".to_string());
    let mint = review
        .token_mint
        .as_deref()
        .map(short_address)
        .unwrap_or_else(|| "native SOL".to_string());
    let content = Text::from(vec![
        Line::from(vec![
            Span::styled("  From:      ", Style::default().fg(theme().fg_dim)),
            Span::styled(
                format!(
                    "{} ({})",
                    review.from_name,
                    short_address(&review.from_address)
                ),
                Style::default().fg(theme().fg),
            ),
        ]),
        Line::from(vec![
            Span::styled("  To:        ", Style::default().fg(theme().fg_dim)),
            Span::styled(&review.to_address, Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled("  Asset:     ", Style::default().fg(theme().fg_dim)),
            Span::styled(
                format!("{} ({})", review.asset_symbol, mint),
                Style::default().fg(theme().fg),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Amount:    ", Style::default().fg(theme().fg_dim)),
            Span::styled(&review.amount_display, Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled("  Network:   ", Style::default().fg(theme().fg_dim)),
            Span::styled(review.network.label(), Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled("  Fee:       ", Style::default().fg(theme().fg_dim)),
            Span::styled(&review.fee_estimate, Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled("  Simulation:", Style::default().fg(theme().fg_dim)),
            Span::styled(
                format!(" {}", simulation),
                Style::default().fg(theme().green),
            ),
        ]),
        Line::from(vec![
            Span::styled("  SPL ATA:   ", Style::default().fg(theme().fg_dim)),
            Span::styled(ata_line, Style::default().fg(theme().fg)),
        ]),
        Line::from(""),
        Line::from("  Press Enter, then type SEND to sign and broadcast. Esc cancels."),
    ]);
    let paragraph = Paragraph::new(content)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme().border))
                .title("Review Transaction"),
        )
        .style(Style::default().fg(theme().fg));
    frame.render_widget(paragraph, area);
}

fn render_receive(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App, width: u16) {
    let (account_name, account_address) = active_account_full(app);

    let receive = Text::from(vec![
        Line::from(format!("Account: {}", account_name)),
        Line::from(format!("Address: {}", account_address)),
        Line::from("Memo: (optional)"),
        Line::from("Press c to copy the receive address."),
        Line::from(if width < QR_MIN_WIDTH {
            "QR hidden at compact widths; widen terminal to scan."
        } else {
            "QR shown below."
        }),
    ]);

    let details = Paragraph::new(receive)
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme().border))
                .title("Receive"),
        )
        .style(Style::default().fg(theme().fg));

    if width < QR_MIN_WIDTH {
        frame.render_widget(details, area);
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(7), Constraint::Min(0)])
        .split(area);

    let qr = Paragraph::new(qr_text(&account_address))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme().border))
                .title("Address QR"),
        )
        .style(Style::default().fg(theme().fg));

    frame.render_widget(details, layout[0]);
    frame.render_widget(qr, layout[1]);
}

fn render_settings(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App, width: u16) {
    let active_name = app
        .accounts
        .iter()
        .find(|a| a.is_active)
        .map(|a| a.name.as_str())
        .unwrap_or("None");
    let wallet_count = app.accounts.len();
    let full_count = app.accounts.iter().filter(|a| a.has_key).count();
    let watch_count = wallet_count - full_count;

    let config = load_den_config();
    let custom_rpc = config
        .network
        .custom_rpc_url
        .as_deref()
        .unwrap_or("not set");
    let custom_rpc_display = if width < COMPACT_WIDTH && custom_rpc != "not set" {
        short_display(custom_rpc, 28)
    } else {
        custom_rpc.to_string()
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(11),
            Constraint::Length(10),
            Constraint::Min(0),
        ])
        .split(area);

    let network_section = Text::from(vec![
        Line::from(vec![
            Span::styled("  Network:    ", Style::default().fg(theme().fg_dim)),
            Span::styled(app.network.label(), Style::default().fg(theme().fg)),
            Span::styled("  (n to toggle)", Style::default().fg(theme().fg_dim)),
        ]),
        Line::from(vec![
            Span::styled("  Default:    ", Style::default().fg(theme().fg_dim)),
            Span::styled(&app.default_network, Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled("  API Key:    ", Style::default().fg(theme().fg_dim)),
            Span::styled(&app.api_key_status, Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled("  Custom RPC: ", Style::default().fg(theme().fg_dim)),
            Span::styled(custom_rpc_display, Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled("  Config:     ", Style::default().fg(theme().fg_dim)),
            Span::styled(&app.config_path_display, Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled("  Data:       ", Style::default().fg(theme().fg_dim)),
            Span::styled(app.refresh_status_label(), Style::default().fg(theme().fg)),
        ]),
    ]);

    let wallet_section = Text::from(vec![
        Line::from(vec![
            Span::styled("  Active:     ", Style::default().fg(theme().fg_dim)),
            Span::styled(active_name, Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled("  Address:    ", Style::default().fg(theme().fg_dim)),
            Span::styled(&app.wallet_address, Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled("  Wallets:    ", Style::default().fg(theme().fg_dim)),
            Span::styled(
                format!(
                    "{} total ({} full, {} watch-only)",
                    wallet_count, full_count, watch_count
                ),
                Style::default().fg(theme().fg),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Last sig:   ", Style::default().fg(theme().fg_dim)),
            Span::styled(&app.last_signature, Style::default().fg(theme().fg)),
        ]),
    ]);

    let shortcuts = Text::from(vec![
        Line::from(vec![
            Span::styled(
                "  n",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  Toggle network", Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled(
                "  r",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  Refresh data", Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled(
                "  i",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  Import wallet", Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled(
                "  s",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  Sign message", Style::default().fg(theme().fg)),
        ]),
        Line::from(vec![
            Span::styled(
                "  o",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  Run setup wizard (Settings tab)",
                Style::default().fg(theme().fg),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  2",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  Manage wallets (Accounts tab)",
                Style::default().fg(theme().fg),
            ),
        ]),
    ]);

    frame.render_widget(
        Paragraph::new(network_section)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme().border))
                    .title("Configuration"),
            )
            .style(Style::default().fg(theme().fg)),
        layout[0],
    );
    frame.render_widget(
        Paragraph::new(wallet_section)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme().border))
                    .title("Wallets"),
            )
            .style(Style::default().fg(theme().fg)),
        layout[1],
    );
    frame.render_widget(
        Paragraph::new(shortcuts)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme().border))
                    .title("Shortcuts"),
            )
            .style(Style::default().fg(theme().fg)),
        layout[2],
    );
}

fn footer_nav_text(tab: Tab, in_detail: bool) -> &'static str {
    match tab {
        Tab::Accounts if in_detail => {
            "Enter:activate | c:copy | x:reveal | e:rename | d:delete | Esc:back | q:quit"
        }
        Tab::Accounts => {
            "Enter:details | c:copy | a:import | g:generate | m:seed | p:restore | w:watch | q:quit"
        }
        Tab::Send if in_detail => "Review: Enter then type SEND to broadcast | Esc:cancel | q:quit",
        Tab::Send => "up/down:asset | Enter:send flow | q:quit",
        Tab::Receive => "c:copy address | QR shown | q:quit",
        Tab::History if in_detail => "c:copy signature | Esc:back | q:quit",
        Tab::History => "Enter:details | c:copy signature | up/down | q:quit",
        Tab::AddressBook if in_detail => {
            "c:copy | e:name | a:address | o:notes | d:delete | Esc:back"
        }
        Tab::AddressBook => "Enter:details | c:copy | a:add | e:edit | d:delete | q:quit",
        _ => "1-8 | up/down | c:copy | n:network | i:import | s:sign | r:refresh | q:quit",
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
                    .border_style(Style::default().fg(theme().border)),
            )
            .style(Style::default().fg(theme().fg));
        frame.render_widget(footer, area);
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let nav = Paragraph::new(nav_text)
        .alignment(Alignment::Center)
        .style(Style::default().fg(theme().fg));
    let status_line = Paragraph::new(status)
        .alignment(Alignment::Center)
        .style(status_style(status));

    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme().border)),
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

fn active_account_full(app: &App) -> (String, String) {
    app.accounts
        .iter()
        .find(|a| a.is_active)
        .map(|a| (a.name.clone(), a.address.clone()))
        .unwrap_or_else(|| ("None".to_string(), "Unset".to_string()))
}

fn copy_to_clipboard(value: &str) -> Result<(), Box<dyn Error>> {
    let commands: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else {
        &[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"])]
    };

    for (program, args) in commands {
        let mut child = match Command::new(program)
            .args(*args)
            .stdin(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(_) => continue,
        };
        if let Some(stdin) = child.stdin.as_mut() {
            stdin.write_all(value.as_bytes())?;
        }
        if child.wait()?.success() {
            return Ok(());
        }
    }

    Err("no supported clipboard command found".into())
}

fn qr_text(value: &str) -> String {
    if value == "Unset" || value.trim().is_empty() {
        return "QR unavailable: no active receive address".to_string();
    }

    match QrCode::new(value.as_bytes()) {
        Ok(code) => code
            .render::<char>()
            .quiet_zone(false)
            .module_dimensions(2, 1)
            .build(),
        Err(_) => "QR unavailable: address could not be encoded".to_string(),
    }
}

fn centered_modal(
    area: Rect,
    max_width: u16,
    preferred_height: u16,
    min_width: u16,
    min_height: u16,
) -> Rect {
    let available_width = area.width.saturating_sub(2).max(1);
    let available_height = area.height.saturating_sub(2).max(1);
    let min_width = min_width.min(available_width).max(1);
    let min_height = min_height.min(available_height).max(1);
    let width = available_width.min(max_width).max(min_width);
    let height = available_height.min(preferred_height).max(min_height);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn render_opaque_modal_background(frame: &mut ratatui::prelude::Frame, area: Rect) {
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default().style(Style::default().bg(theme().surface)),
        area,
    );
}

fn modal_style() -> Style {
    Style::default().fg(theme().fg).bg(theme().surface)
}

fn modal_block(title: &str, border_color: ratatui::style::Color) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme().surface))
        .title(title.to_string())
}

fn render_onboarding_modal(frame: &mut ratatui::prelude::Frame, app: &App) {
    let area = frame.area();

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
            vec!["Enter to continue, Esc to go back".to_string()],
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

    let preferred_height = (lines.len() as u16).saturating_add(2).min(16);
    let modal = centered_modal(area, 86, preferred_height, 24, 8);
    render_opaque_modal_background(frame, modal);

    let paragraph = Paragraph::new(Text::from(lines))
        .wrap(Wrap { trim: false })
        .block(modal_block("Setup", theme().border))
        .style(modal_style());

    frame.render_widget(paragraph, modal);
}

fn render_mnemonic_confirm_modal(frame: &mut ratatui::prelude::Frame, app: &App) {
    let area = frame.area();

    let pending = app.pending_mnemonic.as_ref();
    let phrase = pending
        .map(|pending| pending.mnemonic.as_str())
        .unwrap_or("seed phrase unavailable");
    let name = pending
        .map(|pending| pending.name.as_str())
        .unwrap_or("wallet");
    let derivation = pending
        .map(|pending| pending.derivation_path.as_str())
        .unwrap_or(DEFAULT_DERIVATION_PATH);

    let content = Text::from(vec![
        Line::from(format!("Wallet: {}", name)),
        Line::from(format!("Derivation: {}", derivation)),
        Line::from(""),
        Line::from("Write this 12-word English seed phrase down now."),
        Line::from("Den cannot recover it if you lose it."),
        Line::from(""),
        Line::from(phrase.to_string()),
        Line::from(""),
        Line::from("Type I SAVED IT to store the derived key in Keychain."),
        Line::from(app.input_buffer.clone()),
        Line::from("Esc cancels without storing."),
    ]);

    let modal = centered_modal(area, 92, 15, 32, 8);
    render_opaque_modal_background(frame, modal);

    frame.render_widget(
        Paragraph::new(content)
            .wrap(Wrap { trim: false })
            .block(modal_block("Backup Seed Phrase", theme().yellow))
            .style(modal_style()),
        modal,
    );
}

fn render_revealed_secret_modal(frame: &mut ratatui::prelude::Frame, app: &App) {
    let Some(secret) = &app.revealed_secret else {
        return;
    };
    let area = frame.area();

    let content = Text::from(vec![
        Line::from(format!("Wallet: {}", secret.label)),
        Line::from(format!("Secret type: {}", secret.kind)),
        Line::from(""),
        Line::from("Keep this secret. Anyone with it can spend funds."),
        Line::from(""),
        Line::from(secret.value.clone()),
        Line::from(""),
        Line::from("c copies to clipboard. Esc closes."),
    ]);

    let modal = centered_modal(area, 92, 12, 32, 7);
    render_opaque_modal_background(frame, modal);

    frame.render_widget(
        Paragraph::new(content)
            .wrap(Wrap { trim: false })
            .block(modal_block("Secret Revealed", theme().red))
            .style(modal_style()),
        modal,
    );
}

fn render_input_modal(frame: &mut ratatui::prelude::Frame, app: &App) {
    let area = frame.area();

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
        InputMode::ConfirmDeleteWallet => {
            ("Delete Wallet", delete_prompt, app.input_buffer.clone())
        }
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
        InputMode::SendRecipient => (
            "Send",
            "Enter recipient wallet address:".to_string(),
            app.input_buffer.clone(),
        ),
        InputMode::SendAmount => (
            "Send",
            format!("Enter amount of {}:", app.selected_send_token().symbol),
            app.input_buffer.clone(),
        ),
        InputMode::ConfirmSend => (
            "Confirm Send",
            "Type SEND to sign and broadcast:".to_string(),
            app.input_buffer.clone(),
        ),
        InputMode::GenerateWalletName => (
            "Generate Wallet",
            "Enter a name for the new random keypair:".to_string(),
            app.input_buffer.clone(),
        ),
        InputMode::GenerateMnemonicName => (
            "Generate Seed Wallet",
            "Enter a name for the new 12-word seed wallet:".to_string(),
            app.input_buffer.clone(),
        ),
        InputMode::RestoreMnemonicName => (
            "Restore Seed Wallet",
            "Enter a name for the restored seed wallet:".to_string(),
            app.input_buffer.clone(),
        ),
        InputMode::RestoreMnemonicPhrase => (
            "Restore Seed Wallet",
            "Paste the 12-word English seed phrase:".to_string(),
            "*".repeat(app.input_buffer.len()),
        ),
        InputMode::ConfirmMnemonicSaved => (
            "Backup Seed Phrase",
            "Type I SAVED IT after backing up the phrase:".to_string(),
            app.input_buffer.clone(),
        ),
        InputMode::RevealSecretConfirm => (
            "Reveal Secret",
            "Type REVEAL to display this wallet's secret:".to_string(),
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

    let modal = centered_modal(area, 80, 8, 20, 6);
    render_opaque_modal_background(frame, modal);

    let paragraph = Paragraph::new(content)
        .wrap(Wrap { trim: false })
        .block(modal_block(title, theme().border))
        .style(modal_style());

    frame.render_widget(paragraph, modal);
}

fn status_style(message: &str) -> Style {
    let lower = message.to_ascii_lowercase();
    if lower.contains("error") || lower.contains("failed") || lower.contains("bad") {
        Style::default().fg(theme().accent)
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
        || lower.contains("sent")
        || lower.contains("simulation passed")
    {
        Style::default().fg(theme().green)
    } else {
        Style::default().fg(theme().fg_xdim)
    }
}

fn render_background(frame: &mut ratatui::prelude::Frame, area: Rect) {
    let background = Block::default().style(Style::default().bg(theme().bg));
    frame.render_widget(background, area);
}

fn placeholder_sol_token() -> Token {
    Token {
        symbol: "SOL".to_string(),
        balance: "0.00".to_string(),
        value: "-".to_string(),
        mint: None,
        decimals: 9,
        token_program: None,
    }
}

fn placeholder_transaction() -> Transaction {
    Transaction {
        time: "".to_string(),
        summary: "No transactions".to_string(),
        amount: "".to_string(),
        signature: String::new(),
        slot: 0,
        failed: false,
    }
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
                    key_origin: RAW_KEY_ORIGIN.to_string(),
                    derivation_path: None,
                    added_at: Some(Utc::now().format("%Y-%m-%d").to_string()),
                });
                if config.active_wallet.is_none() {
                    config.active_wallet = Some(wallet_id.clone());
                }
                save_den_config(&config)?;
                println!(
                    "Key imported as '{}' ({}): {}",
                    "Imported",
                    wallet_id,
                    short_address(&address)
                );
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
                    key_origin: RAW_KEY_ORIGIN.to_string(),
                    derivation_path: None,
                    added_at: Some(Utc::now().format("%Y-%m-%d").to_string()),
                });
                if config.active_wallet.is_none() {
                    config.active_wallet = Some(wallet_id.clone());
                }
                save_den_config(&config)?;
                println!(
                    "Added wallet '{}' ({}): {}",
                    name,
                    wallet_id,
                    short_address(&address)
                );
                return Ok(true);
            }
            "--generate-wallet" => {
                let name = args.next().ok_or("Usage: den --generate-wallet <name>")?;
                let address = create_random_wallet(&name)?;
                println!(
                    "Generated wallet '{}' ({}). Use the TUI Accounts tab and type REVEAL to back up the private key.",
                    name,
                    short_address(&address)
                );
                return Ok(true);
            }
            "--restore-mnemonic" => {
                let name = args.next().ok_or("Usage: den --restore-mnemonic <name>")?;
                let phrase =
                    std::env::var("DEN_MNEMONIC").map_err(|_| "DEN_MNEMONIC is not set")?;
                let address = restore_mnemonic_wallet(&name, &phrase)?;
                println!(
                    "Restored mnemonic wallet '{}' ({}) using {}.",
                    name,
                    short_address(&address),
                    DEFAULT_DERIVATION_PATH
                );
                return Ok(true);
            }
            "--add-watch" => {
                let name = args
                    .next()
                    .ok_or("Usage: den --add-watch <name> <address>")?;
                let address = args
                    .next()
                    .ok_or("Usage: den --add-watch <name> <address>")?;
                ensure_config_exists();
                let mut config = load_den_config();
                let wallet_id = next_wallet_id(&config);
                config.wallets.push(WalletEntry {
                    id: wallet_id.clone(),
                    name: name.clone(),
                    address: address.clone(),
                    has_key: false,
                    key_origin: "watch".to_string(),
                    derivation_path: None,
                    added_at: Some(Utc::now().format("%Y-%m-%d").to_string()),
                });
                if config.active_wallet.is_none() {
                    config.active_wallet = Some(wallet_id.clone());
                }
                save_den_config(&config)?;
                println!(
                    "Added watch-only '{}' ({}): {}",
                    name,
                    wallet_id,
                    short_address(&address)
                );
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
                        println!(
                            "{} {} ({}) [{}] {}",
                            marker,
                            w.name,
                            w.id,
                            wtype,
                            short_address(&w.address)
                        );
                    }
                }
                return Ok(true);
            }
            "--remove-wallet" => {
                let target = args
                    .next()
                    .ok_or("Usage: den --remove-wallet <name-or-id>")?;
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
                    let _ = clear_mnemonic_for_wallet(&removed.id);
                }
                if config.active_wallet.as_deref() == Some(&removed.id) {
                    config.active_wallet = config.wallets.first().map(|w| w.id.clone());
                }
                save_den_config(&config)?;
                println!("Removed wallet '{}' ({}).", removed.name, removed.id);
                return Ok(true);
            }
            "--switch-wallet" => {
                let target = args
                    .next()
                    .ok_or("Usage: den --switch-wallet <name-or-id>")?;
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
                let target = args
                    .next()
                    .ok_or("Usage: den --rename-wallet <name-or-id> <new-name>")?;
                let new_name = args
                    .next()
                    .ok_or("Usage: den --rename-wallet <name-or-id> <new-name>")?;
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
                                let _ = clear_mnemonic_for_wallet(&id);
                                if let Some(entry) = config.wallets.iter_mut().find(|e| e.id == id)
                                {
                                    entry.has_key = false;
                                    entry.key_origin = "watch".to_string();
                                    entry.derivation_path = None;
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
            "--set-rpc-url" => {
                let url = args.next().ok_or("Usage: den --set-rpc-url <URL>")?;
                validate_rpc_url(&url)?;
                ensure_config_exists();
                let mut config = load_den_config();
                config.network.custom_rpc_url = Some(url);
                save_den_config(&config)?;
                println!("Custom RPC URL saved. Use: den --set-network custom");
                return Ok(true);
            }
            "--clear-rpc-url" => {
                ensure_config_exists();
                let mut config = load_den_config();
                config.network.custom_rpc_url = None;
                if config.network.default == "custom" {
                    config.network.default = "mainnet".to_string();
                }
                save_den_config(&config)?;
                println!("Custom RPC URL removed.");
                return Ok(true);
            }
            "--set-network" => {
                let net = args
                    .next()
                    .ok_or("Usage: den --set-network <mainnet|devnet|custom>")?;
                match net.as_str() {
                    "mainnet" | "devnet" | "custom" => {
                        ensure_config_exists();
                        let mut config = load_den_config();
                        config.network.default = net;
                        save_den_config(&config)?;
                        println!("Default network saved to config.");
                    }
                    _ => return Err("Network must be 'mainnet', 'devnet', or 'custom'".into()),
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
                println!(
                    "  Custom RPC: {}",
                    config
                        .network
                        .custom_rpc_url
                        .as_deref()
                        .unwrap_or("not set")
                );
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
                    println!(
                        "    {} {} [{}] {}",
                        marker,
                        w.name,
                        wtype,
                        short_address(&w.address)
                    );
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
                let mut invalid = 0u32;
                for contact in incoming.contacts {
                    if validate_solana_address(&contact.address).is_err() {
                        invalid += 1;
                    } else if file
                        .contacts
                        .iter()
                        .any(|c| c.address.trim() == contact.address.trim())
                    {
                        skipped += 1;
                    } else {
                        file.contacts.push(contact);
                        added += 1;
                    }
                }
                save_contacts(&file)?;
                println!(
                    "Imported {} contacts, skipped {} duplicates, ignored {} invalid.",
                    added, skipped, invalid
                );
                return Ok(true);
            }
            "--help" => {
                println!("Den Wallet CLI");
                println!();
                println!("Wallet Management:");
                println!("  --add-wallet NAME       Import key from DEN_SECRET_KEY with name");
                println!("  --generate-wallet NAME  Generate a random keypair wallet");
                println!("  --restore-mnemonic NAME Restore DEN_MNEMONIC at m/44'/501'/0'/0'");
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
                println!(
                    "  --import-contacts FILE  Import contacts from JSON, skip duplicates/invalid addresses"
                );
                println!();
                println!("Configuration:");
                println!("  --set-api-key KEY       Store Helius API key in config");
                println!("  --clear-api-key         Remove API key");
                println!("  --set-network NET       Set default network (mainnet|devnet|custom)");
                println!("  --set-rpc-url URL       Store custom RPC endpoint for custom network");
                println!("  --clear-rpc-url         Remove custom RPC endpoint");
                println!(
                    "  --migrate-config-to-bitwarden [--force]  Copy local config to Bitwarden"
                );
                println!("  --config-path           Show active config location");
                println!("  --status                Show full status");
                return Ok(true);
            }
            _ => {}
        }
    }

    Ok(false)
}

fn create_random_wallet(name: &str) -> Result<String, Box<dyn Error>> {
    ensure_config_exists();
    let keypair = Keypair::new();
    let secret = keypair_to_base58_secret(&keypair);
    let address = keypair.pubkey().to_string();
    let mut config = load_den_config();
    let wallet_id = next_wallet_id(&config);
    store_secret_for_wallet(&wallet_id, &secret)?;
    config.wallets.push(WalletEntry {
        id: wallet_id.clone(),
        name: name.to_string(),
        address: address.clone(),
        has_key: true,
        key_origin: RAW_KEY_ORIGIN.to_string(),
        derivation_path: None,
        added_at: Some(Utc::now().format("%Y-%m-%d").to_string()),
    });
    if config.active_wallet.is_none() {
        config.active_wallet = Some(wallet_id);
    }
    save_den_config(&config)?;
    Ok(address)
}

fn prepare_mnemonic_wallet(name: &str) -> Result<PendingMnemonicWallet, Box<dyn Error>> {
    let mnemonic = Mnemonic::generate_in(Language::English, 12)?;
    let phrase = mnemonic.to_string();
    let keypair = keypair_from_mnemonic_phrase(&phrase, 0)?;
    Ok(PendingMnemonicWallet {
        name: name.to_string(),
        mnemonic: phrase,
        secret: keypair_to_base58_secret(&keypair),
        address: keypair.pubkey().to_string(),
        derivation_path: DEFAULT_DERIVATION_PATH.to_string(),
    })
}

fn restore_mnemonic_wallet(name: &str, phrase: &str) -> Result<String, Box<dyn Error>> {
    let keypair = keypair_from_mnemonic_phrase(phrase, 0)?;
    let pending = PendingMnemonicWallet {
        name: name.to_string(),
        mnemonic: normalize_mnemonic_phrase(phrase)?,
        secret: keypair_to_base58_secret(&keypair),
        address: keypair.pubkey().to_string(),
        derivation_path: DEFAULT_DERIVATION_PATH.to_string(),
    };
    store_mnemonic_wallet(pending)
}

fn store_mnemonic_wallet(pending: PendingMnemonicWallet) -> Result<String, Box<dyn Error>> {
    ensure_config_exists();
    let mut config = load_den_config();
    let wallet_id = next_wallet_id(&config);
    store_secret_for_wallet(&wallet_id, &pending.secret)?;
    store_mnemonic_for_wallet(&wallet_id, &pending.mnemonic)?;
    config.wallets.push(WalletEntry {
        id: wallet_id.clone(),
        name: pending.name,
        address: pending.address.clone(),
        has_key: true,
        key_origin: MNEMONIC_KEY_ORIGIN.to_string(),
        derivation_path: Some(pending.derivation_path),
        added_at: Some(Utc::now().format("%Y-%m-%d").to_string()),
    });
    if config.active_wallet.is_none() {
        config.active_wallet = Some(wallet_id);
    }
    save_den_config(&config)?;
    Ok(pending.address)
}

fn normalize_mnemonic_phrase(phrase: &str) -> Result<String, Box<dyn Error>> {
    let mnemonic = Mnemonic::parse_in_normalized(Language::English, phrase)?;
    Ok(mnemonic.to_string())
}

fn keypair_from_mnemonic_phrase(
    phrase: &str,
    account_index: u32,
) -> Result<Keypair, Box<dyn Error>> {
    let mnemonic = Mnemonic::parse_in_normalized(Language::English, phrase)?;
    let seed = mnemonic.to_seed("");
    let path = DerivationPath::new_bip44(Some(account_index), Some(0));
    keypair_from_seed_and_derivation_path(seed.as_ref(), Some(path))
}

fn keypair_to_base58_secret(keypair: &Keypair) -> String {
    bs58::encode(keypair.to_bytes()).into_string()
}

fn mnemonic_keychain_account(wallet_id: &str) -> String {
    format!("{}:mnemonic", wallet_id)
}

fn store_mnemonic_for_wallet(wallet_id: &str, mnemonic: &str) -> Result<(), Box<dyn Error>> {
    let account = mnemonic_keychain_account(wallet_id);
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, &account)?;
    entry.set_password(mnemonic)?;
    Ok(())
}

fn load_mnemonic_for_wallet(wallet_id: &str) -> Result<String, Box<dyn Error>> {
    let account = mnemonic_keychain_account(wallet_id);
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, &account)?;
    Ok(entry.get_password()?)
}

fn clear_mnemonic_for_wallet(wallet_id: &str) -> Result<(), Box<dyn Error>> {
    let account = mnemonic_keychain_account(wallet_id);
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, &account)?;
    match entry.delete_password() {
        Ok(_) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err.into()),
    }
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
    if config.network.default == "custom" {
        return if config.network.custom_rpc_url.is_some() {
            "API Key: not required for custom RPC".to_string()
        } else {
            "API Key: custom RPC URL not set".to_string()
        };
    }
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

fn build_send_review(
    wallet: &WalletEntry,
    token: &Token,
    recipient: &str,
    amount: &str,
    rpc_url: &str,
    network: Network,
) -> Result<SendReview, Box<dyn Error>> {
    let client = reqwest::blocking::Client::new();
    let from = Pubkey::from_str(&wallet.address)?;
    let to = Pubkey::from_str(recipient)?;
    let raw_amount = decimal_amount_to_raw(amount, token.decimals)?;
    if raw_amount == 0 {
        return Err("amount must be greater than zero".into());
    }
    validate_available_balance(token, raw_amount)?;

    let (instructions, creates_recipient_ata) =
        build_send_instructions(&client, rpc_url, &from, &to, token, raw_amount)?;
    let blockhash = latest_blockhash(&client, rpc_url)?;
    let mut tx = SolanaTransaction::new_with_payer(&instructions, Some(&from));
    tx.message.recent_blockhash = blockhash;
    let simulation_units = simulate_transaction(&client, rpc_url, &tx)?;

    Ok(SendReview {
        from_wallet_id: wallet.id.clone(),
        from_name: wallet.name.clone(),
        from_address: wallet.address.clone(),
        to_address: recipient.to_string(),
        asset_symbol: token.symbol.clone(),
        amount_display: format!("{} {}", amount.trim(), token.symbol),
        raw_amount,
        token_mint: token.mint.clone(),
        token_decimals: token.decimals,
        creates_recipient_ata,
        fee_estimate: "default fee (priority fees disabled)".to_string(),
        simulation_units,
        network,
    })
}

fn broadcast_send(review: &SendReview, rpc_url: &str) -> Result<String, Box<dyn Error>> {
    let client = reqwest::blocking::Client::new();
    let secret = load_secret_for_wallet(&review.from_wallet_id)?;
    let keypair = keypair_from_secret(&secret)?;
    let from = Pubkey::from_str(&review.from_address)?;
    if keypair.pubkey() != from {
        return Err("stored key does not match active wallet address".into());
    }
    let to = Pubkey::from_str(&review.to_address)?;
    let token = Token {
        symbol: review.asset_symbol.clone(),
        balance: String::new(),
        value: String::new(),
        mint: review.token_mint.clone(),
        decimals: review.token_decimals,
        token_program: review
            .token_mint
            .as_ref()
            .map(|_| spl_token::id().to_string()),
    };
    let (instructions, _) =
        build_send_instructions(&client, rpc_url, &from, &to, &token, review.raw_amount)?;
    let blockhash = latest_blockhash(&client, rpc_url)?;
    let tx = SolanaTransaction::new_signed_with_payer(
        &instructions,
        Some(&from),
        &[&keypair],
        blockhash,
    );
    send_transaction(&client, rpc_url, &tx)
}

fn build_send_instructions(
    client: &reqwest::blocking::Client,
    rpc_url: &str,
    from: &Pubkey,
    to: &Pubkey,
    token: &Token,
    raw_amount: u64,
) -> Result<(Vec<Instruction>, bool), Box<dyn Error>> {
    let Some(mint_value) = &token.mint else {
        return Ok((
            vec![system_instruction::transfer(from, to, raw_amount)],
            false,
        ));
    };

    let mint = Pubkey::from_str(mint_value)?;
    let token_program = spl_token::id();
    if let Some(program) = &token.token_program {
        if program != &token_program.to_string() {
            return Err("unsupported token program; Token2022 sends are deferred".into());
        }
    }
    validate_spl_token_mint(client, rpc_url, &mint, &token_program)?;

    let source_ata = get_associated_token_address(from, &mint);
    if !account_exists(client, rpc_url, &source_ata)? {
        return Err(format!("source token account does not exist for {}", token.symbol).into());
    }

    let recipient_ata = get_associated_token_address(to, &mint);
    let creates_recipient_ata = !account_exists(client, rpc_url, &recipient_ata)?;
    let mut instructions = Vec::new();
    if creates_recipient_ata {
        instructions.push(create_associated_token_account_idempotent(
            from,
            to,
            &mint,
            &token_program,
        ));
    }
    instructions.push(spl_token::instruction::transfer_checked(
        &token_program,
        &source_ata,
        &mint,
        &recipient_ata,
        from,
        &[],
        raw_amount,
        token.decimals,
    )?);
    Ok((instructions, creates_recipient_ata))
}

fn validate_spl_token_mint(
    client: &reqwest::blocking::Client,
    rpc_url: &str,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Result<(), Box<dyn Error>> {
    match account_owner(client, rpc_url, mint)? {
        Some(owner) if owner == token_program.to_string() => Ok(()),
        Some(owner) => Err(format!(
            "unsupported token program {}; Token2022 sends are deferred",
            short_address(&owner)
        )
        .into()),
        None => Err("token mint account not found".into()),
    }
}

fn latest_blockhash(
    client: &reqwest::blocking::Client,
    rpc_url: &str,
) -> Result<Hash, Box<dyn Error>> {
    let result = rpc_call(
        client,
        rpc_url,
        "getLatestBlockhash",
        json!([{ "commitment": "confirmed" }]),
    )?;
    let blockhash = result
        .get("value")
        .and_then(|value| value.get("blockhash"))
        .and_then(|value| value.as_str())
        .ok_or("missing latest blockhash")?;
    Ok(Hash::from_str(blockhash)?)
}

fn simulate_transaction(
    client: &reqwest::blocking::Client,
    rpc_url: &str,
    tx: &SolanaTransaction,
) -> Result<Option<u64>, Box<dyn Error>> {
    let encoded = general_purpose::STANDARD.encode(bincode::serialize(tx)?);
    let result = rpc_call(
        client,
        rpc_url,
        "simulateTransaction",
        json!([
            encoded,
            {
                "encoding": "base64",
                "sigVerify": false,
                "replaceRecentBlockhash": true,
                "commitment": "confirmed"
            }
        ]),
    )?;
    if !result
        .get("err")
        .unwrap_or(&serde_json::Value::Null)
        .is_null()
    {
        return Err(format!("simulation failed: {}", result.get("err").unwrap()).into());
    }
    Ok(result.get("unitsConsumed").and_then(|value| value.as_u64()))
}

fn send_transaction(
    client: &reqwest::blocking::Client,
    rpc_url: &str,
    tx: &SolanaTransaction,
) -> Result<String, Box<dyn Error>> {
    let encoded = general_purpose::STANDARD.encode(bincode::serialize(tx)?);
    let result = rpc_call(
        client,
        rpc_url,
        "sendTransaction",
        json!([
            encoded,
            {
                "encoding": "base64",
                "skipPreflight": false,
                "preflightCommitment": "confirmed"
            }
        ]),
    )?;
    result
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "missing transaction signature".into())
}

fn account_exists(
    client: &reqwest::blocking::Client,
    rpc_url: &str,
    pubkey: &Pubkey,
) -> Result<bool, Box<dyn Error>> {
    Ok(account_owner(client, rpc_url, pubkey)?.is_some())
}

fn account_owner(
    client: &reqwest::blocking::Client,
    rpc_url: &str,
    pubkey: &Pubkey,
) -> Result<Option<String>, Box<dyn Error>> {
    let result = rpc_call(
        client,
        rpc_url,
        "getAccountInfo",
        json!([pubkey.to_string(), { "encoding": "base64", "commitment": "confirmed" }]),
    )?;
    let Some(value) = result.get("value") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    Ok(value
        .get("owner")
        .and_then(|owner| owner.as_str())
        .map(str::to_string))
}

fn decimal_amount_to_raw(input: &str, decimals: u8) -> Result<u64, Box<dyn Error>> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.starts_with('-') {
        return Err("amount must be a positive decimal".into());
    }
    let mut parts = trimmed.split('.');
    let whole = parts.next().unwrap_or("0");
    let frac = parts.next().unwrap_or("");
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.chars().all(|ch| ch.is_ascii_digit())
        || !frac.chars().all(|ch| ch.is_ascii_digit())
    {
        return Err("amount must be a positive decimal".into());
    }
    if frac.len() > decimals as usize {
        return Err(format!("amount has more than {} decimal places", decimals).into());
    }
    let scale = 10u128.pow(decimals as u32);
    let whole_raw = whole
        .parse::<u128>()?
        .checked_mul(scale)
        .ok_or("amount too large")?;
    let mut frac_string = frac.to_string();
    while frac_string.len() < decimals as usize {
        frac_string.push('0');
    }
    let frac_raw = if frac_string.is_empty() {
        0
    } else {
        frac_string.parse::<u128>()?
    };
    let raw = whole_raw.checked_add(frac_raw).ok_or("amount too large")?;
    if raw > u64::MAX as u128 {
        return Err("amount too large".into());
    }
    Ok(raw as u64)
}

fn validate_available_balance(token: &Token, raw_amount: u64) -> Result<(), Box<dyn Error>> {
    let balance_raw = decimal_amount_to_raw(&token.balance, token.decimals).unwrap_or(0);
    let reserve = if token.mint.is_none() { 5_000 } else { 0 };
    if raw_amount.saturating_add(reserve) > balance_raw {
        return Err("amount exceeds displayed balance".into());
    }
    Ok(())
}

fn resolve_api_key(config: &DenConfig) -> Option<String> {
    std::env::var("HELIUS_API_KEY")
        .ok()
        .or_else(|| config.network.api_key.clone())
}

fn validate_rpc_url(url: &str) -> Result<(), Box<dyn Error>> {
    let trimmed = url.trim();
    if trimmed.starts_with("https://")
        || trimmed.starts_with("http://localhost")
        || trimmed.starts_with("http://127.0.0.1")
    {
        Ok(())
    } else {
        Err("custom RPC URL must be https:// or local http://localhost/127.0.0.1".into())
    }
}

fn rpc_url_for_network(config: &DenConfig, network: Network) -> Result<String, Box<dyn Error>> {
    match network {
        Network::Mainnet => {
            let api_key = resolve_api_key(config).ok_or("API key not configured")?;
            Ok(format!("https://rpc.helius.xyz/?api-key={}", api_key))
        }
        Network::Devnet => {
            let api_key = resolve_api_key(config).ok_or("API key not configured")?;
            Ok(format!(
                "https://rpc-devnet.helius.xyz/?api-key={}",
                api_key
            ))
        }
        Network::Custom => config
            .network
            .custom_rpc_url
            .clone()
            .ok_or_else(|| "custom RPC URL not configured".into()),
    }
}

fn network_supports_das(network: Network) -> bool {
    !matches!(network, Network::Custom)
}

fn fetch_sol_balance(
    client: &reqwest::blocking::Client,
    rpc_url: &str,
    address: &str,
) -> Result<f64, Box<dyn Error>> {
    let result = rpc_call(client, rpc_url, "getBalance", json!([address]))?;
    let lamports = result.get("value").and_then(|v| v.as_u64()).unwrap_or(0);
    Ok(lamports as f64 / 1_000_000_000.0)
}

fn build_refresh_snapshot(network: Network) -> Result<RefreshSnapshot, Box<dyn Error>> {
    let den_config = load_den_config();
    let keystore_status = keychain_status_summary(&den_config);
    let api_key_status = api_key_status(&den_config);
    let mut wallet_address = "Unset".to_string();
    let mut active_wallet_id = None;
    let mut total_balance = "0.00 SOL".to_string();
    let mut tokens = vec![placeholder_sol_token()];
    let mut nfts = Vec::new();
    let mut history = vec![placeholder_transaction()];

    let rpc_url = match rpc_url_for_network(&den_config, network) {
        Ok(url) => url,
        Err(err) => {
            let accounts = den_config
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
                .collect::<Vec<_>>();

            if let Some(active) = active_wallet(&den_config) {
                wallet_address = short_address(&active.address);
                active_wallet_id = Some(active.id.clone());
            }

            return Ok(RefreshSnapshot {
                accounts,
                active_wallet_id,
                wallet_address,
                total_balance,
                tokens,
                nfts: Vec::new(),
                history,
                keystore_status,
                api_key_status,
                status: format!("Network unavailable: {}", err),
            });
        }
    };
    let client = reqwest::blocking::Client::new();

    let mut accounts = Vec::new();
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

    let status = if let Some(active) = active_wallet(&den_config) {
        let config = Config {
            address: active.address.clone(),
            rpc_url,
            supports_das: network_supports_das(network),
        };
        active_wallet_id = Some(active.id.clone());
        wallet_address = short_address(&active.address);

        match fetch_wallet_data(&config) {
            Ok(data) => {
                total_balance = format!("{:.4} SOL", data.sol_balance);
                if let Some(acc) = accounts.iter_mut().find(|a| a.is_active) {
                    acc.balance = total_balance.clone();
                }
                tokens = data.tokens;
                nfts = data.nfts;
                history = if data.history.is_empty() {
                    vec![placeholder_transaction()]
                } else {
                    data.history
                };
                "Live data from Helius".to_string()
            }
            Err(err) => format!("Helius error: {}", err),
        }
    } else if den_config.wallets.is_empty() {
        "No wallets. Press 'a' on Accounts tab to add one".to_string()
    } else {
        "No active wallet selected".to_string()
    };

    Ok(RefreshSnapshot {
        accounts,
        active_wallet_id,
        wallet_address,
        total_balance,
        tokens,
        nfts,
        history,
        keystore_status,
        api_key_status,
        status,
    })
}

fn fetch_wallet_data(config: &Config) -> Result<WalletData, Box<dyn Error>> {
    let client = reqwest::blocking::Client::new();
    let history = rpc_get_history(&client, &config.rpc_url, &config.address)?;

    if config.supports_das {
        let das_result = das_get_assets(&client, &config.rpc_url, &config.address)?;
        return Ok(WalletData {
            sol_balance: das_result.sol_balance,
            tokens: das_result.tokens,
            nfts: das_result.nfts,
            history,
        });
    }

    let sol_balance = fetch_sol_balance(&client, &config.rpc_url, &config.address)?;
    Ok(WalletData {
        sol_balance,
        tokens: vec![Token {
            symbol: "SOL".to_string(),
            balance: format!("{:.4}", sol_balance),
            value: "-".to_string(),
            mint: None,
            decimals: 9,
            token_program: None,
        }],
        nfts: Vec::new(),
        history,
    })
}

struct DasResult {
    sol_balance: f64,
    tokens: Vec<Token>,
    nfts: Vec<Nft>,
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
        mint: None,
        decimals: 9,
        token_program: None,
    }];

    let mut nfts = Vec::new();

    // Fungible tokens and NFTs from DAS
    if let Some(items) = result.get("items").and_then(|i| i.as_array()) {
        for item in items {
            let interface = item.get("interface").and_then(|i| i.as_str()).unwrap_or("");

            if interface != "FungibleToken" && interface != "FungibleAsset" {
                if let Some(id) = item.get("id").and_then(|id| id.as_str()) {
                    let name = item
                        .get("content")
                        .and_then(|c| c.get("metadata"))
                        .and_then(|m| m.get("name"))
                        .and_then(|s| s.as_str())
                        .unwrap_or("Unnamed NFT")
                        .to_string();
                    let collection = item
                        .get("grouping")
                        .and_then(|g| g.as_array())
                        .and_then(|groups| {
                            groups.iter().find_map(|group| {
                                let label = group.get("group_key").and_then(|v| v.as_str());
                                let value = group.get("group_value").and_then(|v| v.as_str());
                                (label == Some("collection")).then_some(value).flatten()
                            })
                        })
                        .unwrap_or("-")
                        .to_string();
                    nfts.push(Nft {
                        name,
                        collection,
                        address: id.to_string(),
                    });
                }
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
                .unwrap_or_else(|| item.get("id").and_then(|id| id.as_str()).unwrap_or("???"));

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
                mint: item
                    .get("id")
                    .and_then(|id| id.as_str())
                    .map(str::to_string),
                decimals: decimals.min(u8::MAX as u64) as u8,
                token_program: token_info
                    .get("token_program")
                    .and_then(|program| program.as_str())
                    .map(str::to_string),
            });
        }
    }

    Ok(DasResult {
        sol_balance,
        tokens,
        nfts,
    })
}

fn token_program_label(token: &Token) -> String {
    match (&token.mint, token.token_program.as_deref()) {
        (None, _) => "native SOL".to_string(),
        (Some(_), Some(program)) if program == spl_token::id().to_string() => {
            "SPL Token".to_string()
        }
        (Some(_), Some(program)) => format!("unsupported/Token2022? {}", short_address(program)),
        (Some(_), None) => "unknown token program".to_string(),
    }
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
                signature: signature.to_string(),
                slot,
                failed,
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

fn short_display(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars || max_chars <= 1 {
        return value.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    format!("{}…", value.chars().take(keep).collect::<String>())
}

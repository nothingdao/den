use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
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
use solana_sdk::signature::{Keypair, Signer};

const KEYCHAIN_SERVICE: &str = "den-wallet";
const KEYCHAIN_ACCOUNT: &str = "main";

const COLOR_BARK: Color = Color::Rgb(58, 46, 42);
const COLOR_FAWN: Color = Color::Rgb(199, 181, 154);
const COLOR_ASH: Color = Color::Rgb(232, 225, 215);
const COLOR_PINE: Color = Color::Rgb(30, 43, 38);
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
    name: String,
    address: String,
    balance: String,
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
}

struct WalletData {
    sol_balance: f64,
    tokens: Vec<Token>,
    history: Vec<Transaction>,
}

struct Config {
    api_key: String,
    address: String,
    rpc_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ContactsConfig {
    version: u32,
    contacts: Vec<Contact>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputMode {
    None,
    ImportKey,
    SignMessage,
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
    status: String,
    keystore_status: String,
    network: Network,
    input_mode: InputMode,
    input_buffer: String,
    last_signature: String,
}

impl App {
    fn new_placeholder() -> Self {
        let contacts = load_contacts().unwrap_or_else(|_| default_contacts());
        
        Self {
            should_quit: false,
            tab: Tab::Overview,
            accounts: vec![Account {
                name: "Main".to_string(),
                address: "Unset".to_string(),
                balance: "0.00 SOL".to_string(),
            }],
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
            contacts,
            selected_account: 0,
            selected_token: 0,
            selected_history: 0,
            selected_contact: 0,
            total_balance: "0.00 SOL".to_string(),
            wallet_address: "Unset".to_string(),
            status: "Set HELIUS_API_KEY and WALLET_ADDRESS or import a key".to_string(),
            keystore_status: "Keychain: empty".to_string(),
            network: Network::Mainnet,
            input_mode: InputMode::None,
            input_buffer: String::new(),
            last_signature: "-".to_string(),
        }
    }

    fn apply_data(&mut self, address: &str, data: WalletData) {
        self.wallet_address = short_address(address);
        self.total_balance = format!("{:.4} SOL", data.sol_balance);
        self.accounts = vec![Account {
            name: "Main".to_string(),
            address: self.wallet_address.clone(),
            balance: self.total_balance.clone(),
        }];
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
                self.status = format!("Network set to {}", self.network.label());
                refresh_wallet_data(self);
            }
            KeyCode::Char('r') => {
                refresh_wallet_data(self);
            }
            KeyCode::Char('i') => {
                self.input_mode = InputMode::ImportKey;
                self.input_buffer.clear();
            }
            KeyCode::Char('s') => {
                self.input_mode = InputMode::SignMessage;
                self.input_buffer.clear();
            }
            _ => {}
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
                    InputMode::ImportKey => {
                        if input.is_empty() {
                            self.status = "Import cancelled".to_string();
                        } else {
                            match store_secret(&input) {
                                Ok(_) => {
                                    self.status = "Key stored in Keychain".to_string();
                                    self.keystore_status = keychain_status();
                                    refresh_wallet_data(self);
                                }
                                Err(err) => {
                                    self.status = format!("Key import failed: {}", err);
                                }
                            }
                        }
                    }
                    InputMode::SignMessage => {
                        if input.is_empty() {
                            self.status = "Sign cancelled".to_string();
                        } else {
                            match sign_message(&input) {
                                Ok(signature) => {
                                    self.last_signature = signature;
                                    self.status = "Message signed".to_string();
                                }
                                Err(err) => {
                                    self.status = format!("Sign failed: {}", err);
                                }
                            }
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

    // Save contacts before exiting
    let _ = save_contacts(&app.contacts);

    Ok(())
}

fn build_app() -> App {
    let mut app = App::new_placeholder();
    app.keystore_status = keychain_status();

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
        render_footer(frame, layout[2], &app.status, footer_height);
    }

    if app.input_mode != InputMode::None {
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
    let rows = app.accounts.iter().map(|account| {
        Row::new(vec![
            account.name.as_str(),
            account.address.as_str(),
            account.balance.as_str(),
        ])
    });

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Percentage(45),
            Constraint::Percentage(25),
        ],
    )
    .header(
        Row::new(vec!["Name", "Address", "Balance"]).style(
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
            .title("Accounts"),
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
    let items = app
        .contacts
        .iter()
        .map(|contact| {
            let line = format!("{}  {}", contact.name, contact.address);
            ListItem::new(Line::from(line))
        })
        .collect::<Vec<_>>();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BARK))
                .title("Address Book"),
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

fn render_send(frame: &mut ratatui::prelude::Frame, area: Rect, app: &App) {
    let (account_name, account_address) = primary_account(app);

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
    let (account_name, account_address) = primary_account(app);

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
    let settings = Text::from(vec![
        Line::from(format!(
            "Network: {} (press n to toggle)",
            app.network.label()
        )),
        Line::from("Default account: Main"),
        Line::from("Auto-lock: 5 min"),
        Line::from("Export logs: Disabled"),
        Line::from("Theme: Den"),
        Line::from(format!("Wallet address: {}", app.wallet_address)),
        Line::from(app.keystore_status.clone()),
        Line::from("Import key: press i, paste, enter"),
        Line::from("Sign message: press s, enter message"),
        Line::from(format!("Last signature: {}", app.last_signature)),
    ]);

    let paragraph = Paragraph::new(settings)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BARK))
                .title("Settings"),
        )
        .style(Style::default().fg(COLOR_ASH));

    frame.render_widget(paragraph, area);
}

fn render_footer(frame: &mut ratatui::prelude::Frame, area: Rect, status: &str, height: u16) {
    if height == 1 {
        let content = format!(
            "nav: 1-8 switch | up/down list | n network | i import | s sign | r refresh | q quit | {}",
            status
        );
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

    let nav = Paragraph::new(
        "nav: 1-8 switch | up/down list | n network | i import | s sign | r refresh | q quit",
    )
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

fn primary_account(app: &App) -> (String, String) {
    app.accounts
        .first()
        .map(|account| (account.name.clone(), account.address.clone()))
        .unwrap_or_else(|| ("Main".to_string(), "Unset".to_string()))
}

fn render_input_modal(frame: &mut ratatui::prelude::Frame, app: &App) {
    let area = frame.area();
    let modal_width = area.width.saturating_sub(8).min(80).max(20);
    let modal_height = 7u16;
    let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
    let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
    let modal = Rect::new(x, y, modal_width, modal_height);

    let (title, prompt, display) = match app.input_mode {
        InputMode::ImportKey => {
            let masked = "*".repeat(app.input_buffer.len());
            ("Import Key", "Paste secret key and press Enter", masked)
        }
        InputMode::SignMessage => (
            "Sign Message",
            "Enter message and press Enter",
            app.input_buffer.clone(),
        ),
        InputMode::None => ("", "", String::new()),
    };

    let content = Text::from(vec![
        Line::from(prompt),
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
                store_secret(&secret)?;
                println!(
                    "Stored key in macOS Keychain for account '{}'.",
                    KEYCHAIN_ACCOUNT
                );
                return Ok(true);
            }
            "--clear" => {
                clear_secret()?;
                println!(
                    "Removed key from macOS Keychain for account '{}'.",
                    KEYCHAIN_ACCOUNT
                );
                return Ok(true);
            }
            "--add-contact" => {
                let name = args.next().ok_or("--add-contact requires name and address")?;
                let address = args.next().ok_or("--add-contact requires name and address")?;
                
                let mut contacts = load_contacts().unwrap_or_else(|_| default_contacts());
                contacts.push(Contact { name, address });
                save_contacts(&contacts)?;
                println!("Contact added successfully");
                return Ok(true);
            }
            "--remove-contact" => {
                let name = args.next().ok_or("--remove-contact requires contact name")?;
                let mut contacts = load_contacts().unwrap_or_else(|_| default_contacts());
                contacts.retain(|c| c.name != name);
                save_contacts(&contacts)?;
                println!("Contact removed successfully");
                return Ok(true);
            }
            "--list-contacts" => {
                let contacts = load_contacts().unwrap_or_else(|_| default_contacts());
                if contacts.is_empty() {
                    println!("No contacts found");
                } else {
                    println!("Address Book:");
                    for contact in contacts {
                        println!("  {} -> {}", contact.name, contact.address);
                    }
                }
                return Ok(true);
            }
            "--help" => {
                println!("Den Wallet CLI");
                println!("  --import              Store key from DEN_SECRET_KEY in Keychain");
                println!("  --clear               Remove key from Keychain");
                println!("  --add-contact <name> <addr>   Add a contact");
                println!("  --remove-contact <name>       Remove a contact");
                println!("  --list-contacts               List all contacts");
                return Ok(true);
            }
            _ => {}
        }
    }

    Ok(false)
}

fn store_secret(secret: &str) -> Result<(), Box<dyn Error>> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)?;
    entry.set_password(secret)?;
    Ok(())
}

fn clear_secret() -> Result<(), Box<dyn Error>> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)?;
    match entry.delete_password() {
        Ok(_) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn keychain_status() -> String {
    let entry = match keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT) {
        Ok(entry) => entry,
        Err(_) => return "Keychain: unavailable".to_string(),
    };

    match entry.get_password() {
        Ok(_) => "Keychain: stored".to_string(),
        Err(keyring::Error::NoEntry) => "Keychain: empty".to_string(),
        Err(_) => "Keychain: error".to_string(),
    }
}

fn load_secret() -> Result<String, Box<dyn Error>> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)?;
    Ok(entry.get_password()?)
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

fn sign_message(message: &str) -> Result<String, Box<dyn Error>> {
    let secret = load_secret()?;
    let keypair = keypair_from_secret(&secret)?;
    let signature = keypair.sign_message(message.as_bytes());
    Ok(signature.to_string())
}

fn derive_address_from_keychain() -> Result<String, String> {
    let secret = load_secret().map_err(|err| format!("Keychain error: {}", err))?;
    let keypair = keypair_from_secret(&secret).map_err(|err| format!("Bad key: {}", err))?;
    Ok(keypair.pubkey().to_string())
}

fn refresh_wallet_data(app: &mut App) {
    app.keystore_status = keychain_status();
    match Config::from_env(app.network) {
        Ok(config) => {
            app.wallet_address = short_address(&config.address);
            match fetch_wallet_data(&config) {
                Ok(data) => app.apply_data(&config.address, data),
                Err(err) => app.status = format!("Helius error: {}", err),
            }
        }
        Err(message) => app.status = message,
    }
}

impl Config {
    fn from_env(network: Network) -> Result<Self, String> {
        let api_key = std::env::var("HELIUS_API_KEY").ok();
        let address = std::env::var("WALLET_ADDRESS").ok();

        let api_key = api_key.ok_or_else(|| "Set HELIUS_API_KEY for live data".to_string())?;
        let address = match address {
            Some(address) => address,
            None => derive_address_from_keychain()?,
        };

        let rpc_url = match network {
            Network::Mainnet => format!("https://rpc.helius.xyz/?api-key={}", api_key),
            Network::Devnet => format!("https://rpc-devnet.helius.xyz/?api-key={}", api_key),
        };

        Ok(Self {
            api_key,
            address,
            rpc_url,
        })
    }
}

fn fetch_wallet_data(config: &Config) -> Result<WalletData, Box<dyn Error>> {
    let client = reqwest::blocking::Client::new();
    let sol_balance = rpc_get_balance(&client, &config.rpc_url, &config.address)?;
    let mut tokens = Vec::new();

    tokens.push(Token {
        symbol: "SOL".to_string(),
        balance: format!("{:.4}", sol_balance),
        value: "-".to_string(),
        history: seeded_series("SOL", 16),
    });

    let mut token_list = rpc_get_tokens(&client, &config.rpc_url, &config.address)?;
    tokens.append(&mut token_list);

    let history = rpc_get_history(&client, &config.rpc_url, &config.address)?;

    Ok(WalletData {
        sol_balance,
        tokens,
        history,
    })
}

fn rpc_get_balance(
    client: &reqwest::blocking::Client,
    url: &str,
    address: &str,
) -> Result<f64, Box<dyn Error>> {
    let result = rpc_call(client, url, "getBalance", json!([address]))?;
    let lamports = result
        .get("value")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    Ok(lamports as f64 / 1_000_000_000.0)
}

fn rpc_get_tokens(
    client: &reqwest::blocking::Client,
    url: &str,
    address: &str,
) -> Result<Vec<Token>, Box<dyn Error>> {
    let params = json!([
        address,
        { "programId": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA" },
        { "encoding": "jsonParsed" }
    ]);
    let result = rpc_call(client, url, "getTokenAccountsByOwner", params)?;
    let mut tokens = Vec::new();

    if let Some(values) = result.get("value").and_then(|value| value.as_array()) {
        for item in values {
            let info = item
                .get("account")
                .and_then(|account| account.get("data"))
                .and_then(|data| data.get("parsed"))
                .and_then(|parsed| parsed.get("info"));

            let mint = info
                .and_then(|info| info.get("mint"))
                .and_then(|mint| mint.as_str())
                .unwrap_or("Unknown");

            let amount = info
                .and_then(|info| info.get("tokenAmount"))
                .and_then(|amount| amount.get("uiAmountString"))
                .and_then(|amount| amount.as_str())
                .unwrap_or("0");

            let symbol = short_address(mint);
            tokens.push(Token {
                symbol: symbol.clone(),
                balance: amount.to_string(),
                value: "-".to_string(),
                history: seeded_series(&symbol, 16),
            });
        }
    }

    Ok(tokens)
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

fn contacts_config_path() -> Result<PathBuf, Box<dyn Error>> {
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .or_else(|| {
            std::env::var("HOME").ok().map(|home| {
                format!("{}/.config", home)
            })
        })
        .unwrap_or_else(|| ".config".to_string());

    let config_dir = PathBuf::from(config_home).join("den");
    fs::create_dir_all(&config_dir)?;
    Ok(config_dir.join("contacts.json"))
}

fn default_contacts() -> Vec<Contact> {
    vec![
        Contact {
            name: "Trader Joe".to_string(),
            address: "Den9k...9aX1".to_string(),
        },
        Contact {
            name: "Ops Vault".to_string(),
            address: "Den5m...7bN2".to_string(),
        },
        Contact {
            name: "Laptop".to_string(),
            address: "Den2g...2gP8".to_string(),
        },
    ]
}

fn load_contacts() -> Result<Vec<Contact>, Box<dyn Error>> {
    let path = contacts_config_path()?;

    if path.exists() {
        let content = fs::read_to_string(&path)?;
        let config: ContactsConfig = serde_json::from_str(&content)?;
        return Ok(config.contacts);
    }

    // First run: create default config
    let contacts = default_contacts();
    let config = ContactsConfig {
        version: 1,
        contacts: contacts.clone(),
    };
    let content = serde_json::to_string_pretty(&config)?;
    fs::write(&path, content)?;

    Ok(contacts)
}

fn save_contacts(contacts: &[Contact]) -> Result<(), Box<dyn Error>> {
    let path = contacts_config_path()?;
    let config = ContactsConfig {
        version: 1,
        contacts: contacts.to_vec(),
    };
    let content = serde_json::to_string_pretty(&config)?;
    fs::write(&path, content)?;
    Ok(())
}

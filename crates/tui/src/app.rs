//! View state and key handling (DESIGN.md §4 `tui`). Adapted from the
//! `feat/tui-design` visual reference — `Screen`/key bindings carried over,
//! but `App` now holds a real [`DashboardSnapshot`] instead of static
//! fixtures, and announces the selected monitor to the data layer so
//! `Monitor` detail history follows it.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::watch;

use crate::data::DashboardSnapshot;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Screen {
    Fleet,
    Monitor,
    Agents,
    Kubernetes,
    Alerts,
    Health,
    Services,
}

impl Screen {
    // Globe (multi-region latency) is out of this screen set — its data
    // pipeline doesn't exist until Phase 11 (ADR-011); carrying a
    // placeholder tab for it here would just be dead code for two phases.
    pub const ALL: [Self; 7] = [
        Self::Fleet,
        Self::Monitor,
        Self::Agents,
        Self::Kubernetes,
        Self::Alerts,
        Self::Health,
        Self::Services,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Fleet => "FLEET",
            Self::Monitor => "MONITOR",
            Self::Agents => "AGENTS",
            Self::Kubernetes => "K8S",
            Self::Alerts => "ALERTS",
            Self::Health => "HEALTH",
            Self::Services => "SERVICES",
        }
    }
}

pub struct App {
    pub screen: Screen,
    /// Index into `snapshot.monitors` — shared by Fleet (list navigation)
    /// and Monitor (which detail is shown); kept in bounds every time
    /// `snapshot` is refreshed, since the list can shrink between polls.
    pub selected: usize,
    pub show_help: bool,
    pub show_add: bool,
    pub should_quit: bool,
    pub endpoint: String,
    pub snapshot: DashboardSnapshot,
    /// Tells the data layer which monitor's history to fetch (`data.rs`).
    selected_tx: watch::Sender<Option<u64>>,
}

impl App {
    pub fn new(endpoint: &str, selected_tx: watch::Sender<Option<u64>>) -> Self {
        Self {
            screen: Screen::Fleet,
            selected: 0,
            show_help: false,
            show_add: false,
            should_quit: false,
            endpoint: endpoint.to_owned(),
            snapshot: DashboardSnapshot::default(),
            selected_tx,
        }
    }

    /// Called by the render loop whenever a fresh snapshot arrives. Clamps
    /// `selected` and re-announces it — cheap, and simpler than tracking
    /// "did the resolved id actually change" for a watch channel that only
    /// ever holds the latest value anyway.
    pub fn on_snapshot(&mut self, snapshot: DashboardSnapshot) {
        self.snapshot = snapshot;
        if self.selected >= self.snapshot.monitors.len() && !self.snapshot.monitors.is_empty() {
            self.selected = self.snapshot.monitors.len() - 1;
        }
        self.announce_selection();
    }

    fn announce_selection(&self) {
        let id = self.snapshot.monitors.get(self.selected).map(|m| m.id);
        let _ = self.selected_tx.send(id);
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }
        if self.show_help || self.show_add {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                self.show_help = false;
                self.show_add = false;
            }
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('a') if self.screen == Screen::Fleet => self.show_add = true,
            KeyCode::Char(value @ '1'..='7') => {
                self.screen = Screen::ALL[(value as usize) - ('1' as usize)];
            }
            KeyCode::Tab | KeyCode::Right => self.next_screen(),
            KeyCode::BackTab | KeyCode::Left => self.previous_screen(),
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.snapshot.monitors.len();
                if len > 0 {
                    self.selected = (self.selected + 1).min(len - 1);
                    self.announce_selection();
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                self.announce_selection();
            }
            KeyCode::Enter if self.screen == Screen::Fleet => {
                self.screen = Screen::Monitor;
                self.announce_selection();
            }
            KeyCode::Esc => self.screen = Screen::Fleet,
            _ => {}
        }
    }

    fn next_screen(&mut self) {
        let current = Screen::ALL
            .iter()
            .position(|screen| *screen == self.screen)
            .unwrap_or(0);
        self.screen = Screen::ALL[(current + 1) % Screen::ALL.len()];
    }

    fn previous_screen(&mut self) {
        let current = Screen::ALL
            .iter()
            .position(|screen| *screen == self.screen)
            .unwrap_or(0);
        self.screen = Screen::ALL[(current + Screen::ALL.len() - 1) % Screen::ALL.len()];
    }
}

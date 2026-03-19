use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use std::io;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use sysinfo::System;

use super::stats::{BlockInfo, MiningStats};
use crate::mining::MiningEvent;

pub struct Dashboard {
    stats: Arc<Mutex<MiningStats>>,
    system: System,
    event_receiver: Receiver<MiningEvent>,
}

impl Dashboard {
    pub fn new(stats: Arc<Mutex<MiningStats>>, event_receiver: Receiver<MiningEvent>) -> Self {
        let mut system = System::new_all();
        system.refresh_cpu();
        Self {
            stats,
            system,
            event_receiver,
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let res = self.run_app(&mut terminal);

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        res
    }

    fn run_app<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> io::Result<()> {
        loop {
            self.system.refresh_cpu();

            while let Ok(event) = self.event_receiver.try_recv() {
                let mut stats = self.stats.lock().unwrap();
                let block_info = BlockInfo::from_event(event);
                stats.add_block(block_info);
            }

            terminal.draw(|f| self.render(f))?;

            if event::poll(Duration::from_millis(500))? {
                if let Event::Key(key) = event::read()? {
                    if key.code == KeyCode::Char('q') {
                        return Ok(());
                    }
                }
            }
        }
    }

    fn render(&self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Length(8), // CPU
                Constraint::Length(7), // Mining stats
                Constraint::Min(10),   // Recent blocks
                Constraint::Length(1), // Footer
            ])
            .split(f.size());

        self.render_header(f, chunks[0]);
        self.render_cpu(f, chunks[1]);
        self.render_mining_stats(f, chunks[2]);
        self.render_recent_blocks(f, chunks[3]);
        self.render_footer(f, chunks[4]);
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let stats = self.stats.lock().unwrap();
        let uptime = stats.uptime();
        let hours = uptime.as_secs() / 3600;
        let minutes = (uptime.as_secs() % 3600) / 60;

        let text = vec![
            Line::from(vec![Span::styled(
                "FERROUS NETWORK MINING DASHBOARD",
                Style::default().add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![Span::raw(format!(
                "Network: {}  |  Height: {}  |  Uptime: {}h {}m",
                stats.network, stats.current_height, hours, minutes
            ))]),
        ];

        let paragraph = Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL))
            .alignment(Alignment::Center);
        f.render_widget(paragraph, area);
    }

    fn render_cpu(&self, f: &mut Frame, area: Rect) {
        let cpus = self.system.cpus();
        let num_cpus = cpus.len();

        let mut lines = vec![Line::from("CPU USAGE:")];
        lines.push(Line::from(""));

        for (i, cpu) in cpus.iter().enumerate() {
            let usage = cpu.cpu_usage() as u16;
            let bar_width = 40;
            let filled = (usage as usize * bar_width) / 100;
            let bar = "█".repeat(filled) + &"░".repeat(bar_width - filled);

            lines.push(Line::from(vec![
                Span::raw(format!("Core {}: ", i)),
                Span::raw(bar),
                Span::raw(format!(" {}%", usage)),
            ]));
        }

        let total_usage: f32 = cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / num_cpus as f32;
        lines.push(Line::from(""));
        lines.push(Line::from(format!(
            "Total: {}% utilized",
            total_usage as u16
        )));

        let paragraph =
            Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("CPU"));
        f.render_widget(paragraph, area);
    }

    fn render_mining_stats(&self, f: &mut Frame, area: Rect) {
        let stats = self.stats.lock().unwrap();

        let time_since_last = if let Some(last) = stats.last_block_time {
            format!("{:.1}s ago", last.elapsed().as_secs_f64())
        } else {
            "N/A".to_string()
        };

        let last_block_info = stats
            .recent_blocks
            .first()
            .map(|b| format!("Core {} (nonce: {})", b.core, b.nonce))
            .unwrap_or_else(|| "N/A".to_string());

        let hash_rate_str = if stats.hash_rate >= 1_000_000.0 {
            format!("{:.2} MH/s", stats.hash_rate / 1_000_000.0)
        } else if stats.hash_rate >= 1_000.0 {
            format!("{:.2} kH/s", stats.hash_rate / 1_000.0)
        } else if stats.hash_rate > 0.0 {
            format!("{:.0} H/s", stats.hash_rate)
        } else {
            "N/A".to_string()
        };

        let lines = vec![
            Line::from(format!(
                "Blocks Mined:  {} (since start)",
                stats.blocks_mined
            )),
            Line::from(format!("Hash Rate:     {}", hash_rate_str)),
            Line::from(format!("Last Block:    {}", time_since_last)),
            Line::from(format!("Winning Core:  {}", last_block_info)),
            Line::from(format!(
                "Best Hash:     {}",
                if stats.current_hash.len() > 40 {
                    &stats.current_hash[..40]
                } else {
                    &stats.current_hash
                }
            )),
        ];

        let paragraph = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Mining Stats"));
        f.render_widget(paragraph, area);
    }

    fn render_recent_blocks(&self, f: &mut Frame, area: Rect) {
        let stats = self.stats.lock().unwrap();

        let items: Vec<ListItem> = stats
            .recent_blocks
            .iter()
            .map(|block| {
                let time_ago = format!("{:.1}s ago", block.timestamp.elapsed().as_secs_f64());
                let hash_short = if block.hash.len() > 16 {
                    &block.hash[..16]
                } else {
                    &block.hash
                };

                ListItem::new(format!(
                    "#{:<6}  {}...  {:>8}  Core {}  Nonce: {}",
                    block.height, hash_short, time_ago, block.core, block.nonce
                ))
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Recent Blocks"),
        );
        f.render_widget(list, area);
    }

    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let text = Line::from("Press 'q' to quit");
        let paragraph = Paragraph::new(text).alignment(Alignment::Center);
        f.render_widget(paragraph, area);
    }
}

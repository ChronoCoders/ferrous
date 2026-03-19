use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
struct NodeStats {
    name: String,
    ip: String,
    rpc_port: u16,
    blocks: u32,
    headers: u32,
    best_hash: String,
    chain: String,
    difficulty: f64,
    hash_rate: f64,
    peer_count: Option<u32>,
    connections: Option<u32>,
    connections_in: Option<u32>,
    connections_out: Option<u32>,
    send_rate_mbps: Option<f64>,
    recv_rate_mbps: Option<f64>,
    uptime_secs: Option<u64>,
    partition_detected: Option<bool>,
    recovery_attempts: Option<u32>,
    last_block_age_secs: Option<u64>,
    rpc_rtt_ms: Option<u64>,
    recent_blocks: Vec<RecentBlock>,
    reachable: bool,
    last_updated: Instant,
    mining_error: Option<String>,
    blocks_error: Option<String>,
    network_error: Option<String>,
    recovery_error: Option<String>,
}

#[derive(Clone)]
struct RecentBlock {
    height: u32,
    hash: String,
    time_ago_secs: u64,
}

struct NodeUpdate {
    name: String,
    stats: NodeStats,
}

fn main() -> io::Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<NodeUpdate>();

    let poller_1 = spawn_poller(
        tx.clone(),
        stop.clone(),
        "seed1".to_string(),
        "45.77.153.141".to_string(),
        18331,
    );
    let poller_4 = spawn_poller(
        tx,
        stop.clone(),
        "seed4".to_string(),
        "45.77.64.221".to_string(),
        18332,
    );

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut seed1 = default_node_stats("seed1", "45.77.153.141", 18331);
    let mut seed4 = default_node_stats("seed4", "45.77.64.221", 18332);
    seed1.reachable = is_port_reachable(18331);
    seed4.reachable = is_port_reachable(18332);
    let mut last_updated_at = SystemTime::now();

    let res = run_app(
        &mut terminal,
        &rx,
        &mut seed1,
        &mut seed4,
        &mut last_updated_at,
    );

    stop.store(true, Ordering::Relaxed);
    let _ = poller_1.join();
    let _ = poller_4.join();

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
    terminal: &mut Terminal<B>,
    rx: &mpsc::Receiver<NodeUpdate>,
    seed1: &mut NodeStats,
    seed4: &mut NodeStats,
    last_updated_at: &mut SystemTime,
) -> io::Result<()> {
    loop {
        while let Ok(update) = rx.try_recv() {
            if update.name == seed1.name {
                *seed1 = update.stats;
            } else if update.name == seed4.name {
                *seed4 = update.stats;
            }
            *last_updated_at = SystemTime::now();
        }

        terminal.draw(|f| render(f, seed1, seed4, *last_updated_at))?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') {
                    return Ok(());
                }
            }
        }
    }
}

fn render(f: &mut Frame, seed1: &NodeStats, seed4: &NodeStats, last_updated_at: SystemTime) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(10),
            Constraint::Length(8),
            Constraint::Length(1),
        ])
        .split(f.size());

    render_nodes_row(f, chunks[0], seed1, seed4);
    render_summary_row(f, chunks[1], seed1, seed4);
    render_footer(f, chunks[2], last_updated_at);
}

fn render_nodes_row(f: &mut Frame, area: Rect, seed1: &NodeStats, seed4: &NodeStats) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_node_panel(f, cols[0], seed1);
    render_node_panel(f, cols[1], seed4);
}

fn render_node_panel(f: &mut Frame, area: Rect, node: &NodeStats) {
    let title = format!("{} ({})", node.name, node.ip);
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(12), Constraint::Min(3)])
        .split(inner);

    let status = if node.reachable { "ONLINE" } else { "OFFLINE" };
    let status_style = if node.reachable {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    };

    let sync_line = if node.blocks < node.headers {
        Line::from(vec![
            Span::styled("SYNCING", Style::default().fg(Color::Yellow)),
            Span::raw(format!(" ({}/{})", node.blocks, node.headers)),
        ])
    } else {
        Line::from(Span::styled("SYNCED", Style::default().fg(Color::Green)))
    };

    let peers_line = match node.peer_count {
        Some(n) => format!("{}", n),
        None => "N/A".to_string(),
    };

    let best_hash_short = if node.best_hash.len() > 24 {
        &node.best_hash[..24]
    } else {
        &node.best_hash
    };

    let diff_line = if node.difficulty.is_finite() {
        format!("{:.8}", node.difficulty)
    } else {
        "N/A".to_string()
    };

    let hash_rate_line = if node.hash_rate.is_finite() {
        format!("{:.2} H/s", node.hash_rate)
    } else {
        "N/A".to_string()
    };

    let mut lines: Vec<Line> = vec![Line::from(vec![
        Span::raw("Status: "),
        Span::styled(status, status_style),
    ])];

    if !node.reachable {
        lines.push(Line::from(""));
        lines.push(Line::from("Tunnel not active. Run:"));
        lines.push(Line::from(format!(
            "ssh -N -L {}:127.0.0.1:8332 root@{}",
            node.rpc_port, node.ip
        )));
    } else {
        if let Some(err) = &node.mining_error {
            lines.push(Line::from(Span::styled(
                err.as_str(),
                Style::default().fg(Color::Yellow),
            )));
        }
        if let Some(err) = &node.blocks_error {
            lines.push(Line::from(Span::styled(
                err.as_str(),
                Style::default().fg(Color::Yellow),
            )));
        }
        if let Some(err) = &node.network_error {
            lines.push(Line::from(Span::styled(
                err.as_str(),
                Style::default().fg(Color::Yellow),
            )));
        }
        if let Some(err) = &node.recovery_error {
            lines.push(Line::from(Span::styled(
                err.as_str(),
                Style::default().fg(Color::Yellow),
            )));
        }
        lines.push(Line::from(format!(
            "Chain: {}  Blocks: {}",
            node.chain, node.blocks
        )));
        lines.push(sync_line);
        lines.push(Line::from(format!("Peers: {}", peers_line)));
        lines.push(Line::from(format!("Best: {}...", best_hash_short)));
        lines.push(Line::from(format!("Difficulty: {}", diff_line)));
        lines.push(Line::from(format!("Hash rate: {}", hash_rate_line)));
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, chunks[0]);

    let items: Vec<ListItem> = if node.recent_blocks.is_empty() {
        vec![ListItem::new("Fetching...")]
    } else {
        node.recent_blocks
            .iter()
            .map(|b| {
                let hash_short = if b.hash.len() > 16 {
                    &b.hash[..16]
                } else {
                    &b.hash
                };
                ListItem::new(format!(
                    "#{:<6} {}...  {:>6}s ago",
                    b.height, hash_short, b.time_ago_secs
                ))
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Recent Blocks"),
    );
    f.render_widget(list, chunks[1]);
}

fn render_summary_row(f: &mut Frame, area: Rect, seed1: &NodeStats, seed4: &NodeStats) {
    let height_diff = seed1.blocks.abs_diff(seed4.blocks);
    let tip_match = seed1.reachable
        && seed4.reachable
        && !seed1.best_hash.is_empty()
        && (seed1.blocks != seed4.blocks || seed1.best_hash == seed4.best_hash);

    let combined_peers = match (seed1.peer_count, seed4.peer_count) {
        (Some(a), Some(b)) => Some(a.saturating_add(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        _ => None,
    };

    let peers_line = match combined_peers {
        Some(n) => n.to_string(),
        None => "N/A".to_string(),
    };

    let tip_status = if tip_match { "MATCH" } else { "FORK" };
    let tip_style = if tip_match {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    };

    let last_block_seed1 = seed1
        .last_block_age_secs
        .or_else(|| seed1.recent_blocks.first().map(|b| b.time_ago_secs));
    let last_block_seed4 = seed4
        .last_block_age_secs
        .or_else(|| seed4.recent_blocks.first().map(|b| b.time_ago_secs));

    let last_block_line = format!(
        "Last block age (s): seed1={}  seed4={}",
        last_block_seed1
            .map(|s| s.to_string())
            .unwrap_or_else(|| "N/A".to_string()),
        last_block_seed4
            .map(|s| s.to_string())
            .unwrap_or_else(|| "N/A".to_string())
    );

    let rec_seed1 = match seed1.partition_detected {
        Some(true) => format!(
            "seed1=PARTITION attempts={} age={}",
            seed1.recovery_attempts.unwrap_or(0),
            seed1.last_block_age_secs.unwrap_or(0)
        ),
        Some(false) => format!(
            "seed1=OK attempts={} age={}",
            seed1.recovery_attempts.unwrap_or(0),
            seed1.last_block_age_secs.unwrap_or(0)
        ),
        None => "seed1=N/A".to_string(),
    };
    let rec_seed4 = match seed4.partition_detected {
        Some(true) => format!(
            "seed4=PARTITION attempts={} age={}",
            seed4.recovery_attempts.unwrap_or(0),
            seed4.last_block_age_secs.unwrap_or(0)
        ),
        Some(false) => format!(
            "seed4=OK attempts={} age={}",
            seed4.recovery_attempts.unwrap_or(0),
            seed4.last_block_age_secs.unwrap_or(0)
        ),
        None => "seed4=N/A".to_string(),
    };

    let net_seed1 = match seed1.connections {
        Some(c) => format!("seed1 conn={}", c),
        None => "seed1 conn=N/A".to_string(),
    };
    let net_seed4 = match seed4.connections {
        Some(c) => format!("seed4 conn={}", c),
        None => "seed4 conn=N/A".to_string(),
    };

    let rtt_seed1 = seed1
        .rpc_rtt_ms
        .map(|v| format!("seed1={}ms", v))
        .unwrap_or_else(|| "seed1=N/A".to_string());
    let rtt_seed4 = seed4
        .rpc_rtt_ms
        .map(|v| format!("seed4={}ms", v))
        .unwrap_or_else(|| "seed4=N/A".to_string());

    let lines = vec![
        Line::from(vec![
            Span::styled(
                "Network Summary",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  Tip: "),
            Span::styled(tip_status, tip_style),
        ]),
        Line::from(format!(
            "seed1: {}  |  seed4: {}  |  Difference: {}",
            seed1.blocks, seed4.blocks, height_diff
        )),
        Line::from(format!(
            "Peers: seed1={} seed4={}  |  Combined: {}",
            seed1
                .peer_count
                .map(|v| v.to_string())
                .unwrap_or_else(|| "N/A".to_string()),
            seed4
                .peer_count
                .map(|v| v.to_string())
                .unwrap_or_else(|| "N/A".to_string()),
            peers_line
        )),
        Line::from(format!("P2P connections: {}  {}", net_seed1, net_seed4)),
        Line::from(last_block_line),
        Line::from(format!("Recovery: {}  {}", rec_seed1, rec_seed4)),
        Line::from(format!("RPC RTT: {}  {}", rtt_seed1, rtt_seed4)),
    ];

    let paragraph = Paragraph::new(lines).block(Block::default().borders(Borders::ALL));
    f.render_widget(paragraph, area);
}

fn render_footer(f: &mut Frame, area: Rect, last_updated_at: SystemTime) {
    let line = Line::from(vec![
        Span::raw(format!(
            "Last updated: {}  |  ",
            format_hhmmss(last_updated_at)
        )),
        Span::styled(
            "Press q to quit",
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ]);
    let paragraph = Paragraph::new(line);
    f.render_widget(paragraph, area);
}

fn spawn_poller(
    tx: mpsc::Sender<NodeUpdate>,
    stop: Arc<AtomicBool>,
    name: String,
    ip: String,
    local_port: u16,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut last_good = default_node_stats(&name, &ip, local_port);
        let mut consecutive_failures: u32 = 0;
        let mut last_poll_at = Instant::now();

        while !stop.load(Ordering::Relaxed) {
            let now = Instant::now();
            let delta_secs = now
                .duration_since(last_poll_at)
                .as_secs()
                .min(u64::from(u32::MAX));
            last_poll_at = now;

            let (stats, ok) = poll_node(&name, &ip, local_port, Some((&last_good, delta_secs)));
            if ok {
                consecutive_failures = 0;
                last_good = stats.clone();
                let _ = tx.send(NodeUpdate {
                    name: name.clone(),
                    stats,
                });
            } else {
                consecutive_failures = consecutive_failures.saturating_add(1);
                if consecutive_failures < 3 && last_good.reachable {
                    // Keep showing last good data, just update error
                    let mut carry = last_good_snapshot(&last_good, delta_secs);
                    carry.mining_error = stats.mining_error.clone();
                    carry.blocks_error = stats.blocks_error.clone();
                    let _ = tx.send(NodeUpdate {
                        name: name.clone(),
                        stats: carry,
                    });
                } else {
                    let _ = tx.send(NodeUpdate {
                        name: name.clone(),
                        stats,
                    });
                }
            }

            // Poll every 5 seconds
            for _ in 0..50 {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    })
}

fn poll_node(
    name: &str,
    ip: &str,
    local_port: u16,
    prior: Option<(&NodeStats, u64)>,
) -> (NodeStats, bool) {
    let mut stats = default_node_stats(name, ip, local_port);
    stats.last_updated = Instant::now();

    if !is_port_reachable(local_port) {
        stats.reachable = false;
        return (stats, false);
    }

    // getblockchaininfo
    let t0 = Instant::now();
    let info = match rpc_call(local_port, "getblockchaininfo", Value::Array(vec![])) {
        Ok(v) => v,
        Err(e) => {
            stats.reachable = false;
            stats.blocks_error = Some(format!("RPC: {}", e));
            return (stats, false);
        }
    };
    stats.rpc_rtt_ms = Some(t0.elapsed().as_millis() as u64);

    stats.reachable = true;
    stats.chain = info
        .get("chain")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    stats.blocks = info.get("blocks").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    stats.headers = info.get("headers").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    stats.best_hash = info
        .get("bestblockhash")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // getpeerinfo
    if let Ok(v) = rpc_call(local_port, "getpeerinfo", Value::Array(vec![])) {
        if let Some(arr) = v.as_array() {
            stats.peer_count = Some(arr.len() as u32);
        } else if let Some(obj) = v.as_object() {
            if let Some(count) = obj.get("count").and_then(|c| c.as_u64()) {
                stats.peer_count = Some(count as u32);
            } else if let Some(peers) = obj.get("peers").and_then(|p| p.as_array()) {
                stats.peer_count = Some(peers.len() as u32);
            }
        }
    }

    // getmininginfo — difficulty and hashrate come from here
    match rpc_call(local_port, "getmininginfo", Value::Array(vec![])) {
        Ok(v) => {
            stats.difficulty = v
                .get("difficulty")
                .and_then(|d| d.as_f64())
                .unwrap_or(f64::NAN);
            stats.hash_rate = v
                .get("networkhashps")
                .or_else(|| v.get("hashrate"))
                .or_else(|| v.get("hash_rate"))
                .and_then(|h| h.as_f64())
                .unwrap_or(f64::NAN);
            stats.mining_error = None;
        }
        Err(e) => {
            stats.mining_error = Some(format!("Mining: getmininginfo unavailable ({})", e));

            if !stats.best_hash.is_empty() {
                if let Ok(block) = rpc_call(local_port, "getblock", json!([stats.best_hash])) {
                    if let Some(bits_hex) = block.get("bits").and_then(|b| b.as_str()) {
                        if let Ok(bits) = u32::from_str_radix(bits_hex, 16) {
                            if let Some(d) = difficulty_from_compact(bits) {
                                stats.difficulty = d;
                                stats.hash_rate = d * 4294967296.0 / 150.0;
                                stats.mining_error = None;
                            }
                        }
                    }
                }
            }
        }
    }

    match rpc_call(local_port, "getnetworkinfo", Value::Array(vec![])) {
        Ok(v) => {
            stats.connections = v
                .get("connections")
                .and_then(|n| n.as_u64())
                .map(|n| n as u32);
            stats.connections_in = v
                .get("connections_in")
                .and_then(|n| n.as_u64())
                .map(|n| n as u32);
            stats.connections_out = v
                .get("connections_out")
                .and_then(|n| n.as_u64())
                .map(|n| n as u32);
            stats.send_rate_mbps = v.get("send_rate_mbps").and_then(|n| n.as_f64());
            stats.recv_rate_mbps = v.get("recv_rate_mbps").and_then(|n| n.as_f64());
            stats.uptime_secs = v.get("uptime").and_then(|n| n.as_u64());
            stats.network_error = None;
        }
        Err(e) => {
            stats.network_error = Some(format!("Network: getnetworkinfo unavailable ({})", e));
        }
    }

    match rpc_call(local_port, "getrecoverystatus", Value::Array(vec![])) {
        Ok(v) => {
            stats.partition_detected = v.get("partition_detected").and_then(|b| b.as_bool());
            stats.recovery_attempts = v
                .get("recovery_attempts")
                .and_then(|n| n.as_u64())
                .map(|n| n as u32);
            stats.last_block_age_secs = v.get("last_block_age").and_then(|n| n.as_u64());
            stats.recovery_error = None;
        }
        Err(e) => {
            stats.recovery_error = Some(format!("Recovery: getrecoverystatus unavailable ({})", e));
        }
    }

    // Recent blocks — only refresh when height changes, non-blocking failures kept separate
    let need_refresh = match prior {
        Some((prev, _)) => prev.blocks != stats.blocks || prev.recent_blocks.is_empty(),
        None => true,
    };

    if need_refresh {
        // Try to fetch recent blocks — failure here does NOT make node OFFLINE
        match fetch_recent_blocks(local_port, stats.blocks, 10) {
            Ok(recent) => {
                stats.recent_blocks = recent;
                stats.blocks_error = None;
            }
            Err(e) => {
                // Keep last known recent blocks if available
                if let Some((prev, delta)) = prior {
                    if !prev.recent_blocks.is_empty() {
                        stats.recent_blocks = prev
                            .recent_blocks
                            .iter()
                            .map(|b| RecentBlock {
                                height: b.height,
                                hash: b.hash.clone(),
                                time_ago_secs: b.time_ago_secs.saturating_add(delta),
                            })
                            .collect();
                    }
                }
                stats.blocks_error = Some(format!("Blocks: {}", e));
            }
        }
    } else if let Some((prev, delta)) = prior {
        stats.recent_blocks = prev
            .recent_blocks
            .iter()
            .map(|b| RecentBlock {
                height: b.height,
                hash: b.hash.clone(),
                time_ago_secs: b.time_ago_secs.saturating_add(delta),
            })
            .collect();
        stats.blocks_error = None;
    }

    (stats, true)
}

fn fetch_recent_blocks(
    local_port: u16,
    tip_height: u32,
    count: usize,
) -> Result<Vec<RecentBlock>, String> {
    let now = now_unix_secs();
    let heights: Vec<u32> = (0..count)
        .map(|i| tip_height.saturating_sub(i as u32))
        .collect();

    let mut reqs = Vec::with_capacity(heights.len());
    for (i, height) in heights.iter().copied().enumerate() {
        reqs.push(json!({
            "jsonrpc": "2.0",
            "method": "getblockhash",
            "params": [height],
            "id": (i as u64) + 1
        }));
    }

    let responses = rpc_batch(local_port, Value::Array(reqs))?;
    let mut by_id: HashMap<u64, Value> = HashMap::with_capacity(responses.len());
    for r in responses {
        if let Some(id) = r.get("id").and_then(|v| v.as_u64()) {
            by_id.insert(id, r);
        }
    }

    let mut hashes: Vec<String> = Vec::with_capacity(heights.len());
    for i in 0..heights.len() {
        let id = (i as u64) + 1;
        let resp = by_id
            .get(&id)
            .ok_or_else(|| format!("Missing batch response for id {}", id))?;
        if resp.get("error").is_some() {
            return Err(resp.to_string());
        }
        let hash = resp
            .get("result")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if hash.is_empty() {
            break;
        }
        hashes.push(hash);
    }

    let mut block_reqs = Vec::with_capacity(hashes.len());
    for (i, hash) in hashes.iter().enumerate() {
        block_reqs.push(json!({
            "jsonrpc": "2.0",
            "method": "getblock",
            "params": [hash],
            "id": 1000u64 + (i as u64)
        }));
    }

    let block_responses = rpc_batch(local_port, Value::Array(block_reqs))?;
    let mut blocks_by_id: HashMap<u64, Value> = HashMap::with_capacity(block_responses.len());
    for r in block_responses {
        if let Some(id) = r.get("id").and_then(|v| v.as_u64()) {
            blocks_by_id.insert(id, r);
        }
    }

    let mut out: Vec<RecentBlock> = Vec::with_capacity(blocks_by_id.len());
    for (i, height) in heights.iter().copied().enumerate() {
        let id = 1000u64 + (i as u64);
        let resp = match blocks_by_id.get(&id) {
            Some(r) => r,
            None => break,
        };
        if resp.get("error").is_some() {
            return Err(resp.to_string());
        }

        let block = resp
            .get("result")
            .cloned()
            .ok_or_else(|| "Missing result".to_string())?;

        let block_hash = block
            .get("hash")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let block_height = block
            .get("height")
            .and_then(|v| v.as_u64())
            .unwrap_or(height as u64);
        let time = block.get("time").and_then(|v| v.as_u64()).unwrap_or(0);
        let time_ago = now.saturating_sub(time);

        out.push(RecentBlock {
            height: block_height as u32,
            hash: block_hash,
            time_ago_secs: time_ago,
        });
    }

    Ok(out)
}

fn default_node_stats(name: &str, ip: &str, rpc_port: u16) -> NodeStats {
    NodeStats {
        name: name.to_string(),
        ip: ip.to_string(),
        rpc_port,
        blocks: 0,
        headers: 0,
        best_hash: String::new(),
        chain: "unknown".to_string(),
        difficulty: f64::NAN,
        hash_rate: f64::NAN,
        peer_count: None,
        connections: None,
        connections_in: None,
        connections_out: None,
        send_rate_mbps: None,
        recv_rate_mbps: None,
        uptime_secs: None,
        partition_detected: None,
        recovery_attempts: None,
        last_block_age_secs: None,
        rpc_rtt_ms: None,
        recent_blocks: Vec::new(),
        reachable: false,
        last_updated: Instant::now(),
        mining_error: None,
        blocks_error: None,
        network_error: None,
        recovery_error: None,
    }
}

fn last_good_snapshot(prev: &NodeStats, delta_secs: u64) -> NodeStats {
    let mut s = default_node_stats(&prev.name, &prev.ip, prev.rpc_port);
    s.blocks = prev.blocks;
    s.headers = prev.headers;
    s.best_hash = prev.best_hash.clone();
    s.chain = prev.chain.clone();
    s.difficulty = prev.difficulty;
    s.hash_rate = prev.hash_rate;
    s.peer_count = prev.peer_count;
    s.connections = prev.connections;
    s.connections_in = prev.connections_in;
    s.connections_out = prev.connections_out;
    s.send_rate_mbps = prev.send_rate_mbps;
    s.recv_rate_mbps = prev.recv_rate_mbps;
    s.uptime_secs = prev.uptime_secs;
    s.partition_detected = prev.partition_detected;
    s.recovery_attempts = prev.recovery_attempts;
    s.last_block_age_secs = prev
        .last_block_age_secs
        .map(|a| a.saturating_add(delta_secs));
    s.rpc_rtt_ms = prev.rpc_rtt_ms;
    s.reachable = prev.reachable;
    s.last_updated = prev.last_updated;
    s.recent_blocks = prev
        .recent_blocks
        .iter()
        .map(|b| RecentBlock {
            height: b.height,
            hash: b.hash.clone(),
            time_ago_secs: b.time_ago_secs.saturating_add(delta_secs),
        })
        .collect();
    s
}

fn rpc_call(local_port: u16, method: &str, params: Value) -> Result<Value, String> {
    let req = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1
    });
    let resp = http_post_json(local_port, &req.to_string())?;

    if resp.get("error").is_some() {
        return Err(resp.to_string());
    }

    resp.get("result")
        .cloned()
        .ok_or_else(|| "Missing result".to_string())
}

fn rpc_batch(local_port: u16, requests: Value) -> Result<Vec<Value>, String> {
    let resp = http_post_json(local_port, &requests.to_string())?;
    match resp {
        Value::Array(items) => Ok(items),
        other => Ok(vec![other]),
    }
}

fn http_post_json(local_port: u16, body: &str) -> Result<Value, String> {
    let mut stream =
        TcpStream::connect_timeout(&local_socket_addr(local_port), Duration::from_secs(5))
            .map_err(|e| format!("{:?}", e))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(20)));

    let request = format!(
        "POST / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
        port = local_port,
        len = body.len(),
        body = body
    );

    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("{:?}", e))?;
    stream.flush().map_err(|e| format!("{:?}", e))?;

    let mut resp_bytes = Vec::new();
    stream
        .read_to_end(&mut resp_bytes)
        .map_err(|e| format!("{:?}", e))?;

    let resp_str = String::from_utf8_lossy(&resp_bytes);
    let mut parts = resp_str.splitn(2, "\r\n\r\n");
    let header = parts.next().unwrap_or("");
    let body = parts.next().unwrap_or("");

    if !header.contains("200") {
        return Err(format!(
            "HTTP error: {}",
            header.lines().next().unwrap_or("")
        ));
    }

    serde_json::from_str(body).map_err(|e| format!("JSON parse error: {:?}", e))
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs()
}

fn format_hhmmss(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs();
    let day_secs = secs % 86_400;
    let h = day_secs / 3600;
    let m = (day_secs % 3600) / 60;
    let s = day_secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

fn local_socket_addr(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

fn is_port_reachable(port: u16) -> bool {
    TcpStream::connect_timeout(&local_socket_addr(port), Duration::from_secs(1)).is_ok()
}

fn difficulty_from_compact(bits: u32) -> Option<f64> {
    if bits == 0 {
        return None;
    }
    let exponent = ((bits >> 24) & 0xff) as i32;
    let mantissa_u32 = bits & 0x00ff_ffff;
    if mantissa_u32 == 0 {
        return None;
    }

    let mantissa = mantissa_u32 as f64;
    let target = mantissa * 2f64.powi(8 * (exponent - 3));

    let diff1_mantissa = 0x0000ffffu32 as f64;
    let diff1_exponent = 0x1d_i32;
    let diff1_target = diff1_mantissa * 2f64.powi(8 * (diff1_exponent - 3));

    Some(diff1_target / target)
}

//! Detailed event log (`events.log` next to the game's saves): for each tick, the input,
//! frame changes, every event that fired with the objects it picked, and what each of its
//! actions changed (values, positions, visibility, counters, text, created / destroyed
//! objects, global values). Effects are read from the objects before and after each
//! action, never by evaluating parameters again (that would advance the random numbers
//! and change the game).
//!
//! Kept for the last N minutes of play (written when the session ends, on Ctrl+L, when
//! the game pauses after errors, and into problem reports) or for the whole session
//! (streamed to the file as it happens).

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::value::Value;
use super::Runtime;

/// How much of the log to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    /// The last N minutes of play.
    Minutes(u32),
    /// Everything, written to the file as it happens.
    Session,
}

impl Keep {
    /// "off" / "" / "0" = no log, "minute" = 1, a number = that many minutes, "session" /
    /// "all" = the whole session.
    pub fn parse(s: &str) -> Option<Keep> {
        let s = s.trim().to_ascii_lowercase();
        match s.as_str() {
            "" | "off" | "0" | "no" => None,
            "minute" | "on" | "1" => Some(Keep::Minutes(1)),
            "session" | "all" | "full" => Some(Keep::Session),
            n => n.trim_end_matches("min").trim().parse::<u32>().ok().filter(|&m| m > 0).map(|m| Keep::Minutes(m.min(600))),
        }
    }

    /// The settings / launcher choice: 0 off, 1 last minute, 2 last 5 minutes, 3 session.
    pub fn from_choice(k: u8) -> Option<Keep> {
        match k {
            1 => Some(Keep::Minutes(1)),
            2 => Some(Keep::Minutes(5)),
            3 => Some(Keep::Session),
            _ => None,
        }
    }

    fn describe(&self) -> String {
        match self {
            Keep::Minutes(1) => "the last minute".into(),
            Keep::Minutes(m) => format!("the last {m} minutes"),
            Keep::Session => "the whole session".into(),
        }
    }
}

pub struct EventLog {
    keep: Keep,
    path: PathBuf,
    /// (tick played since the session started, line)
    lines: VecDeque<(u64, String)>,
    /// Header lines (kept even when old lines are dropped).
    head: Vec<String>,
    file: Option<std::io::BufWriter<std::fs::File>>,
    ticks_per_minute: u64,
    /// The tick whose header was written last (lines are grouped by tick).
    header_tick: Option<u64>,
    /// Ticks played this session (frames restart their own tick count).
    pub played: u64,
    /// Times each event fired this tick: after `REPEATS`, its lines are left out and
    /// counted instead (fast loops run the same events hundreds of times a tick).
    fired: std::collections::HashMap<usize, u32>,
    /// Lines are being left out (a repeated event's actions).
    pub(super) muted: bool,
    /// Size of the kept lines, and whether old ones were dropped to stay under MAX_BYTES.
    bytes: usize,
    capped: bool,
    /// Also log what objects' own movements and animations did each tick.
    pub detailed: bool,
}

/// At most this many objects' movement / animation changes are listed per tick.
const MOVES_PER_TICK: usize = 40;

/// How many times an event is logged in full per tick.
const REPEATS: u32 = 5;

/// Memory kept for "last N minutes" logs: in games that change hundreds of objects every
/// tick, the oldest lines go first once this is reached (the file says so).
const MAX_BYTES: usize = 64 << 20;

impl EventLog {
    pub fn new(keep: Keep, path: PathBuf, frame_rate: u32) -> Self {
        let file = if keep == Keep::Session {
            let _ = path.parent().map(std::fs::create_dir_all);
            std::fs::File::create(&path).ok().map(std::io::BufWriter::new)
        } else {
            None
        };
        let mut log = EventLog {
            keep,
            path,
            lines: VecDeque::new(),
            head: Vec::new(),
            file,
            ticks_per_minute: frame_rate.max(1) as u64 * 60,
            header_tick: None,
            played: 0,
            fired: std::collections::HashMap::new(),
            muted: false,
            bytes: 0,
            capped: false,
            detailed: std::env::var("KLIKCORE_EVENT_LOG_DETAIL").is_ok_and(|v| v != "0"),
        };
        let head = format!(
            "KlikCore {} event log ({}), started {}",
            env!("CARGO_PKG_VERSION"),
            keep.describe(),
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
        for l in [head, "Event numbers match `klikcore <game> --dump-events <frame>`.".to_string()] {
            if log.file.is_some() {
                log.raw(l);
            } else {
                log.head.push(l);
            }
        }
        log
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn raw(&mut self, line: String) {
        match (&mut self.file, self.keep) {
            (Some(f), _) => {
                let _ = writeln!(f, "{line}");
            }
            (None, Keep::Minutes(m)) => {
                self.bytes += line.len() + 24;
                self.lines.push_back((self.played, line));
                let keep_from = self.played.saturating_sub(m as u64 * self.ticks_per_minute);
                while let Some((t, l)) = self.lines.front() {
                    let over = self.bytes > MAX_BYTES;
                    if *t >= keep_from && !over {
                        break;
                    }
                    self.capped |= over;
                    self.bytes -= l.len() + 24;
                    self.lines.pop_front();
                }
            }
            (None, Keep::Session) => {}
        }
    }

    /// Add a line under the header of tick `tick` of frame `frame`.
    pub fn line(&mut self, frame: usize, tick: u64, timer_ms: f64, line: String) {
        if self.muted {
            return;
        }
        if self.header_tick != Some(self.played) {
            self.end_tick();
            self.header_tick = Some(self.played);
            self.raw(format!("--- frame {frame} tick {tick} ({:.2} s)", timer_ms / 1000.0));
        }
        self.raw(line);
    }

    /// An event fired: false (and its lines muted) once it already fired `REPEATS` times
    /// this tick.
    pub(super) fn event_fired(&mut self, gi: usize) -> bool {
        if self.header_tick != Some(self.played) {
            self.end_tick();
        }
        let n = self.fired.entry(gi).or_default();
        *n += 1;
        self.muted = *n > REPEATS;
        !self.muted
    }

    /// Say how often the repeated events fired in the tick that just ended.
    fn end_tick(&mut self) {
        let mut many: Vec<(usize, u32)> = self.fired.drain().filter(|&(_, n)| n > REPEATS).collect();
        many.sort_unstable();
        for (gi, n) in many {
            self.raw(format!("  (event #{gi} fired {n} times this tick; the first {REPEATS} are shown)"));
        }
    }

    /// Write the log to its file: the kept lines, or flush the streamed session.
    pub fn save(&mut self) -> std::io::Result<()> {
        let muted = std::mem::replace(&mut self.muted, false);
        self.end_tick();
        self.muted = muted;
        if let Some(f) = &mut self.file {
            return f.flush();
        }
        let _ = self.path.parent().map(std::fs::create_dir_all);
        let mut f = std::io::BufWriter::new(std::fs::File::create(&self.path)?);
        for l in &self.head {
            writeln!(f, "{l}")?;
        }
        if self.capped {
            writeln!(f, "(This game changes so much every tick that only the last {} MB are kept: the log covers less time than asked.)", MAX_BYTES >> 20)?;
        }
        for (_, l) in &self.lines {
            writeln!(f, "{l}")?;
        }
        f.flush()
    }
}

impl Drop for EventLog {
    fn drop(&mut self) {
        if let Some(f) = &mut self.file {
            let _ = f.flush();
        }
    }
}

/// What an action could change about one object.
#[derive(Clone, PartialEq)]
pub(super) struct ObjSnap {
    idx: usize,
    x: f64,
    y: f64,
    visible: bool,
    destroyed: bool,
    layer: usize,
    anim: usize,
    values: Vec<Value>,
    strings: Vec<String>,
    counter: Value,
    text: String,
    flags: u32,
}

/// The state an action could change: its objects, the global values and strings, score and
/// lives, and how many objects exist (to spot new ones).
pub(super) struct Snap {
    objs: Vec<ObjSnap>,
    globals: Vec<Value>,
    gstrings: Vec<String>,
    score: [i32; 4],
    lives: [i32; 4],
    count: usize,
}

/// Alterable value / string names as Fusion shows them (A-Z, then numbers).
fn slot(i: usize) -> String {
    if i < 26 {
        ((b'A' + i as u8) as char).to_string()
    } else {
        (i + 1).to_string()
    }
}

fn short(s: &str) -> String {
    let s = s.replace('\n', "\\n").replace('\r', "");
    if s.chars().count() > 60 {
        format!("{:?}...", s.chars().take(60).collect::<String>())
    } else {
        format!("{s:?}")
    }
}

impl Runtime {
    /// Start the event log (`keep`), in `events.log` in the game's save folder.
    /// `KLIKCORE_EVENT_LOG_FILE` puts it elsewhere (headless runs and scenarios use a
    /// temporary save folder that is removed afterwards).
    pub fn start_event_log(&mut self, keep: Keep) {
        let path = std::env::var_os("KLIKCORE_EVENT_LOG_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.files.save_root.join("events.log"));
        self.event_log = Some(EventLog::new(keep, path, self.frame_rate));
    }

    /// Before the objects' own movements and animations run: where everything is (only
    /// for a detailed log).
    pub(super) fn motion_snapshot(&self) -> Option<Vec<(u32, f64, f64, usize)>> {
        if !self.event_log.as_ref().is_some_and(|l| l.detailed) {
            return None;
        }
        Some(self.instances.iter().map(|i| (i.id, i.x, i.y, i.current_anim_id())).collect())
    }

    /// After them: log who moved and whose animation changed or ended.
    pub(super) fn log_motion(&mut self, before: Vec<(u32, f64, f64, usize)>) {
        let mut lines: Vec<String> = Vec::new();
        let mut more = 0;
        for (k, (id, x, y, anim)) in before.into_iter().enumerate() {
            let Some(inst) = self.instances.get(k) else { break };
            if inst.id != id || inst.destroyed {
                continue;
            }
            let mut d: Vec<String> = Vec::new();
            if (inst.x, inst.y) != (x, y) {
                d.push(format!("({x:.0}, {y:.0}) -> ({:.0}, {:.0})", inst.x, inst.y));
            }
            if inst.current_anim_id() != anim {
                d.push(format!("animation {anim} -> {}", inst.current_anim_id()));
            }
            if let Some(a) = inst.anim.finished_anim {
                d.push(format!("animation {a} ended"));
            }
            if d.is_empty() {
                continue;
            }
            if lines.len() >= MOVES_PER_TICK {
                more += 1;
                continue;
            }
            lines.push(format!("  moves: {} {}", self.object_label(k), d.join(", ")));
        }
        if more > 0 {
            lines.push(format!("  moves: ... and {more} more objects"));
        }
        for l in lines {
            self.elog(l);
        }
    }

    /// Write the event log now; returns its file.
    pub fn save_event_log(&mut self) -> Option<std::io::Result<PathBuf>> {
        let log = self.event_log.as_mut()?;
        Some(log.save().map(|_| log.path().to_path_buf()))
    }

    /// Note the frame the game is on (when the log starts after the frame was loaded).
    pub fn log_frame_start(&mut self, frame: usize) {
        let name = self.game.frames.get(frame).map(|f| f.name.clone()).unwrap_or_default();
        self.elog(format!("== frame {frame} \"{name}\""));
    }

    pub(super) fn elog(&mut self, line: String) {
        let (frame, tick, timer) = (self.frame_index, self.tick, self.timer_ms);
        if let Some(log) = &mut self.event_log {
            log.line(frame, tick, timer, line);
        }
    }

    pub(super) fn object_label(&self, idx: usize) -> String {
        let inst = &self.instances[idx];
        let name = self.game.object(inst.oi).map_or_else(|| format!("object {}", inst.oi), |o| o.name.clone());
        format!("\"{name}\"#{}", inst.id)
    }

    /// What the event's conditions were, for the log: owner, number, NOT.
    pub(super) fn describe_conditions(&self, gi: usize) -> String {
        let Some(g) = self.events.groups.get(gi) else { return String::new() };
        g.conditions
            .iter()
            .map(|c| {
                let owner = if c.object_type < 0 {
                    self.ace_owner_label(c.object_type)
                } else {
                    self.game.object(c.object_info).map_or_else(|| format!("#{}", c.object_info), |o| format!("\"{}\"", o.name))
                };
                format!("{}{owner} {}", if c.negated() { "NOT " } else { "" }, c.num)
            })
            .collect::<Vec<_>>()
            .join(" & ")
    }

    pub(super) fn snapshot(&self, targets: &[usize]) -> Snap {
        let objs = targets
            .iter()
            .filter_map(|&i| {
                let inst = self.instances.get(i)?;
                Some(ObjSnap {
                    idx: i,
                    x: inst.x,
                    y: inst.y,
                    visible: inst.visible,
                    destroyed: inst.destroyed,
                    layer: inst.layer,
                    anim: inst.current_anim_id(),
                    values: inst.values.clone(),
                    strings: inst.strings.clone(),
                    counter: inst.counter.clone(),
                    text: inst.text.clone(),
                    flags: inst.flags,
                })
            })
            .collect();
        Snap {
            objs,
            globals: self.global_values.clone(),
            gstrings: self.global_strings.clone(),
            score: self.score,
            lives: self.lives,
            count: self.instances.len(),
        }
    }

    /// What changed since `before`, as log lines.
    pub(super) fn change_lines(&self, before: &Snap) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let after = self.snapshot(&before.objs.iter().map(|o| o.idx).collect::<Vec<_>>());
        let mut changed_objs = 0;
        for (b, a) in before.objs.iter().zip(&after.objs) {
            if b == a {
                continue;
            }
            changed_objs += 1;
            if changed_objs > 20 {
                continue;
            }
            let mut d: Vec<String> = Vec::new();
            if a.destroyed && !b.destroyed {
                d.push("destroyed".into());
            }
            if (b.x, b.y) != (a.x, a.y) {
                d.push(format!("position ({:.0}, {:.0}) -> ({:.0}, {:.0})", b.x, b.y, a.x, a.y));
            }
            if b.visible != a.visible {
                d.push(if a.visible { "shown".into() } else { "hidden".into() });
            }
            if b.layer != a.layer {
                d.push(format!("layer {} -> {}", b.layer + 1, a.layer + 1));
            }
            if b.anim != a.anim {
                d.push(format!("animation {} -> {}", b.anim, a.anim));
            }
            for i in 0..b.values.len().max(a.values.len()) {
                let (x, y) = (b.values.get(i).cloned().unwrap_or(Value::Int(0)), a.values.get(i).cloned().unwrap_or(Value::Int(0)));
                if x != y {
                    d.push(format!("value {} {x} -> {y}", slot(i)));
                }
            }
            for i in 0..b.strings.len().max(a.strings.len()) {
                let (x, y) = (b.strings.get(i).cloned().unwrap_or_default(), a.strings.get(i).cloned().unwrap_or_default());
                if x != y {
                    d.push(format!("string {} {} -> {}", slot(i), short(&x), short(&y)));
                }
            }
            if b.counter != a.counter {
                d.push(format!("counter {} -> {}", b.counter, a.counter));
            }
            for bit in 0..32 {
                let (was, now) = (b.flags >> bit & 1, a.flags >> bit & 1);
                if was != now {
                    d.push(format!("flag {bit} {}", if now == 1 { "on" } else { "off" }));
                }
            }
            if b.text != a.text {
                d.push(format!("text {} -> {}", short(&b.text), short(&a.text)));
            }
            out.push(format!("      {}: {}", self.object_label(b.idx), d.join(", ")));
        }
        if changed_objs > 20 {
            out.push(format!("      ... and {} more objects changed", changed_objs - 20));
        }
        for i in 0..before.globals.len().max(self.global_values.len()) {
            let (x, y) = (before.globals.get(i).cloned().unwrap_or(Value::Int(0)), self.global_values.get(i).cloned().unwrap_or(Value::Int(0)));
            if x != y {
                out.push(format!("      global value {} {x} -> {y}", i + 1));
            }
        }
        for i in 0..before.gstrings.len().max(self.global_strings.len()) {
            let (x, y) = (before.gstrings.get(i).cloned().unwrap_or_default(), self.global_strings.get(i).cloned().unwrap_or_default());
            if x != y {
                out.push(format!("      global string {} {} -> {}", i + 1, short(&x), short(&y)));
            }
        }
        for p in 0..4 {
            if before.score[p] != self.score[p] {
                out.push(format!("      player {} score {} -> {}", p + 1, before.score[p], self.score[p]));
            }
            if before.lives[p] != self.lives[p] {
                out.push(format!("      player {} lives {} -> {}", p + 1, before.lives[p], self.lives[p]));
            }
        }
        for i in before.count..self.instances.len() {
            let inst = &self.instances[i];
            out.push(format!("      created {} at ({:.0}, {:.0}) layer {}", self.object_label(i), inst.x, inst.y, inst.layer + 1));
        }
        out
    }
}


/// Split an object's change list ("value A 1 -> 2, position (1, 2) -> (3, 4), text "a, b"
/// -> "c"") on the commas between changes, not those inside brackets or quotes.
fn split_changes(s: &str) -> Vec<&str> {
    let (mut parts, mut start, mut depth, mut quoted, mut escaped) = (Vec::new(), 0, 0i32, false, false);
    let b = s.as_bytes();
    for i in 0..b.len() {
        let c = b[i];
        if quoted {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => quoted = false,
                _ => {}
            }
            continue;
        }
        match c {
            b'"' => quoted = true,
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth == 0 && b.get(i + 1) == Some(&b' ') => {
                parts.push(&s[start..i]);
                start = i + 2;
            }
            _ => {}
        }
    }
    parts.push(&s[start.min(s.len())..]);
    parts
}

/// Look through an event log for signs of trouble (in the game or in KlikCore) and sum
/// them up: objects created and never removed, values flipping back and forth every
/// tick, the same thing changed several times in one tick, objects that wandered very far
/// away, and huge values. For problem reports and `--analyze-log`.
pub fn analyze(text: &str) -> String {
    use std::collections::HashMap;
    let mut tick: u64 = 0;
    let mut ticks = 0u64;
    let mut frames: Vec<String> = Vec::new();
    let mut created: HashMap<String, u32> = HashMap::new();
    let mut destroyed: HashMap<String, u32> = HashMap::new();
    // object id -> name, for objects created and not destroyed yet
    let mut alive: HashMap<String, String> = HashMap::new();
    // (name, id, field) -> [(tick, old, new)]
    let mut changes: HashMap<(String, String, String), Vec<(u64, String, String)>> = HashMap::new();
    let mut far: HashMap<String, (u32, String)> = HashMap::new();
    let mut huge: Vec<String> = Vec::new();
    let mut repeats: HashMap<String, (u32, u32)> = HashMap::new();

    // `"name"#id` at the start of `s`: (name, id, rest).
    fn object(s: &str) -> Option<(&str, &str, &str)> {
        let s = s.strip_prefix('"')?;
        let q = s.find("\"#")?;
        let (name, rest) = (&s[..q], &s[q + 2..]);
        let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        Some((name, &rest[..end], &rest[end..]))
    }
    fn number(s: &str) -> Option<f64> {
        s.trim().trim_matches(|c| c == '(' || c == ')').split(',').next()?.trim().parse().ok()
    }

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("--- frame ") {
            ticks += 1;
            if let Some(t) = rest.split(" tick ").nth(1).and_then(|t| t.split_whitespace().next()).and_then(|t| t.parse().ok()) {
                tick = t;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("== frame ") {
            frames.push(rest.to_string());
            continue;
        }
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("(event #") {
            // "(event #N fired M times this tick; ...)"
            let mut w = rest.split_whitespace();
            if let (Some(n), Some(m)) = (w.next(), w.nth(1)) {
                let e = repeats.entry(n.to_string()).or_default();
                e.0 += 1;
                e.1 = e.1.max(m.parse().unwrap_or(0));
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("created ") {
            if let Some((name, id, _)) = object(rest) {
                *created.entry(name.to_string()).or_default() += 1;
                alive.insert(id.to_string(), name.to_string());
            }
            continue;
        }
        let body = t.strip_prefix("moves: ").unwrap_or(t);
        let Some((name, id, rest)) = object(body) else { continue };
        let rest = rest.trim_start_matches(':').trim();
        for part in split_changes(rest) {
            if part == "destroyed" {
                *destroyed.entry(name.to_string()).or_default() += 1;
                alive.remove(id);
                continue;
            }
            if part == "shown" || part == "hidden" {
                changes.entry((name.to_string(), id.to_string(), "visibility".into())).or_default().push((tick, String::new(), part.to_string()));
                continue;
            }
            let Some((head, new)) = part.split_once(" -> ") else { continue };
            // "value C 0", "counter 3", "position (1, 2)", "(1, 2)" (a move), "animation 0"
            let (field, old) = if head.starts_with('(') {
                ("position".to_string(), head.to_string())
            } else if let Some(k) = head.find(['(', '"']) {
                (head[..k].trim().to_string(), head[k..].to_string())
            } else {
                match head.rsplit_once(' ') {
                    Some((f, o)) => (f.to_string(), o.to_string()),
                    None => (head.to_string(), String::new()),
                }
            };
            if field == "position" {
                if let Some((x, y)) = new.trim_matches(|c| c == '(' || c == ')').split_once(", ") {
                    let (x, y) = (x.trim().parse::<f64>().unwrap_or(0.0), y.trim().parse::<f64>().unwrap_or(0.0));
                    if x.abs() > 20_000.0 || y.abs() > 20_000.0 {
                        let e = far.entry(name.to_string()).or_default();
                        e.0 += 1;
                        e.1 = format!("({x:.0}, {y:.0})");
                    }
                }
            } else if let Some(v) = number(new) {
                if v.abs() > 1e8 && huge.len() < 10 {
                    huge.push(format!("{name}#{id} {field} = {new} (tick {tick})"));
                }
            }
            changes.entry((name.to_string(), id.to_string(), field)).or_default().push((tick, old, new.to_string()));
        }
    }

    let mut out = String::new();
    let section = |title: &str, lines: Vec<String>, out: &mut String| {
        if !lines.is_empty() {
            out.push_str(&format!("\n{title}\n"));
            for l in lines {
                out.push_str(&format!("  {l}\n"));
            }
        }
    };
    out.push_str(&format!("Event log summary: {ticks} ticks logged, frames: {}\n", if frames.is_empty() { "-".into() } else { frames.join(", ") }));

    // Created during the log and never removed (by an action).
    let mut left: HashMap<&str, u32> = HashMap::new();
    for n in alive.values() {
        *left.entry(n.as_str()).or_default() += 1;
    }
    let mut left: Vec<(&str, u32)> = left.into_iter().filter(|&(_, k)| k >= 3).collect();
    left.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    section(
        "Objects created and never removed (3 or more; they may pile up, or leave the screen):",
        left.iter().take(10).map(|(n, k)| format!("{k} \"{n}\" (created {}, removed {})", created.get(*n).unwrap_or(&0), destroyed.get(*n).unwrap_or(&0))).collect(),
        &mut out,
    );

    // A -> B -> A on consecutive ticks.
    let mut flips: Vec<(u32, String)> = Vec::new();
    for ((name, id, field), seq) in &changes {
        let n = seq
            .windows(2)
            .filter(|w| w[0].2 == w[1].1 && w[1].2 == w[0].1 && w[0].1 != w[0].2 && w[1].0 <= w[0].0 + 1)
            .count() as u32;
        if n >= 20 {
            let last = seq.last().cloned().unwrap_or_default();
            flips.push((n, format!("\"{name}\"#{id} {field}: {} <-> {} ({n} times)", last.1, last.2)));
        }
    }
    flips.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    section(
        "Values flipping back and forth every tick (random choices do this; otherwise events may be fighting):",
        flips.into_iter().take(10).map(|(_, l)| l).collect(),
        &mut out,
    );

    // Changed more than once within one tick (visibility is left out: hiding everything
    // and then showing some is common).
    let mut fights: HashMap<(String, String), u32> = HashMap::new();
    for ((name, _, field), seq) in &changes {
        if field == "visibility" {
            continue;
        }
        let mut per: HashMap<u64, u32> = HashMap::new();
        for (t, _, _) in seq {
            *per.entry(*t).or_default() += 1;
        }
        let k = per.values().filter(|&&c| c > 1).count() as u32;
        if k > 0 {
            *fights.entry((name.clone(), field.clone())).or_default() += k;
        }
    }
    let mut fights: Vec<((String, String), u32)> = fights.into_iter().filter(|(_, k)| *k >= 30).collect();
    fights.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    section(
        "Changed several times within one tick, on many ticks (events undoing each other?):",
        fights.into_iter().take(10).map(|((n, f), k)| format!("\"{n}\" {f}: on {k} ticks")).collect(),
        &mut out,
    );

    let mut far: Vec<(String, (u32, String))> = far.into_iter().collect();
    far.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.0.cmp(&b.0)));
    section(
        "Objects very far from the frame (over 20000 pixels away):",
        far.into_iter().take(10).map(|(n, (k, at))| format!("\"{n}\" {k} times, last at {at}")).collect(),
        &mut out,
    );
    section("Huge values:", huge, &mut out);
    let mut rep: Vec<(String, (u32, u32))> = repeats.into_iter().collect();
    rep.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.0.cmp(&b.0)));
    section(
        "Events repeated many times within a tick (fast loops):",
        rep.into_iter().take(8).map(|(n, (k, m))| format!("event #{n}: on {k} ticks, up to {m} times a tick")).collect(),
        &mut out,
    );
    if !out.contains("\n\n") {
        out.push_str("\nNothing unusual found.\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_finds_leaks_and_flips() {
        let mut log = String::from("KlikCore event log\n== frame 0 \"f\"\n");
        for t in 0..60u64 {
            log += &format!("--- frame 0 tick {t} (0.00 s)\n");
            log += &format!("  event #1: System -1\n    Create action 0\n      created \"spark\"#{} at (1, 2) layer 1\n", 100 + t);
            let (a, b) = if t % 2 == 0 { (1, 4) } else { (4, 1) };
            log += &format!("    \"pick\" action 80\n      \"pick\"#7: counter {a} -> {b}\n");
        }
        log += "      \"spark\"#100: destroyed\n";
        let s = analyze(&log);
        assert!(s.contains("59 \"spark\" (created 60, removed 1)"), "{s}");
        assert!(s.contains("\"pick\"#7 counter"), "{s}");
        assert!(analyze("--- frame 0 tick 0 (0.00 s)\n").contains("Nothing unusual"));
    }

    #[test]
    fn keep_choices() {
        assert_eq!(Keep::parse("off"), None);
        assert_eq!(Keep::parse("minute"), Some(Keep::Minutes(1)));
        assert_eq!(Keep::parse("5"), Some(Keep::Minutes(5)));
        assert_eq!(Keep::parse("10min"), Some(Keep::Minutes(10)));
        assert_eq!(Keep::parse("session"), Some(Keep::Session));
        assert_eq!(Keep::from_choice(2), Some(Keep::Minutes(5)));
    }

    #[test]
    fn minute_log_drops_old_lines() {
        let dir = std::env::temp_dir().join(format!("klikcore-elog-test-{}", std::process::id()));
        let mut log = EventLog::new(Keep::Minutes(1), dir.join("events.log"), 60);
        for t in 0..(60 * 60 * 3) {
            log.played = t;
            log.line(0, t, 0.0, format!("line {t}"));
        }
        // Only about the last minute (3600 ticks, header + line each) is kept.
        assert!(log.lines.len() <= 2 * 3601, "{}", log.lines.len());
        assert!(log.lines.back().is_some_and(|(_, l)| l == "line 10799"));
        log.save().unwrap();
        let text = std::fs::read_to_string(dir.join("events.log")).unwrap();
        assert!(text.starts_with("KlikCore") && text.contains("line 10799") &&!text.contains("line 100\n"));
        let _ = std::fs::remove_dir_all(dir);
    }
}

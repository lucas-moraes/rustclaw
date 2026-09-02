//! Animation primitives: spinners, particles, splash, aurora.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use super::theme::Theme;

/// Braille spinner frames.
pub const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Claw-ish alternate spinner.
pub const CLAW_SPIN: &[&str] = &["ᕙ", "ᕗ", "ᕕ", "ᕙ", "ᕗ", "ᕕ"];

/// Streaming / cursor blink glyphs.
pub const CURSOR_ON: &str = "▍";
pub const CURSOR_OFF: &str = " ";

pub fn spinner_frame(tick: u64) -> &'static str {
    SPINNER[(tick as usize / 2) % SPINNER.len()]
}

pub fn claw_frame(tick: u64) -> &'static str {
    CLAW_SPIN[(tick as usize / 3) % CLAW_SPIN.len()]
}

pub fn cursor_glyph(tick: u64) -> &'static str {
    if (tick / 6) % 2 == 0 {
        CURSOR_ON
    } else {
        CURSOR_OFF
    }
}

pub fn thinking_dots(tick: u64) -> String {
    let n = ((tick / 5) % 4) as usize;
    format!("thinking{}", ".".repeat(n))
}

/// Floating particle for header / splash.
#[derive(Clone, Debug)]
pub struct Particle {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub life: u16,
    pub max_life: u16,
    pub ch: char,
    pub color_idx: u8,
}

impl Particle {
    pub fn spawn(width: u16, height: u16, tick: u64) -> Self {
        let seed = tick.wrapping_mul(1103515245).wrapping_add(12345);
        let x = (seed % width.max(1) as u64) as f32;
        let y = ((seed >> 8) % height.max(1) as u64) as f32;
        let chars = ['·', '✦', '*', '·', '✧', '.'];
        let ch = chars[(seed as usize >> 4) % chars.len()];
        let life = 40 + ((seed >> 12) % 40) as u16;
        Self {
            x,
            y,
            vx: (((seed >> 16) % 7) as f32 - 3.0) * 0.15,
            vy: 0.08 + ((seed >> 20) % 5) as f32 * 0.04,
            life,
            max_life: life,
            ch,
            color_idx: ((seed >> 24) % 3) as u8,
        }
    }

    pub fn tick(&mut self) {
        self.x += self.vx;
        self.y += self.vy;
        self.life = self.life.saturating_sub(1);
    }

    pub fn alive(&self) -> bool {
        self.life > 0
    }

    pub fn style(&self, theme: &Theme) -> Style {
        let c = match self.color_idx {
            0 => theme.accent,
            1 => theme.accent2,
            _ => theme.accent3,
        };
        // Fade by life remaining.
        if self.life < self.max_life / 4 {
            Style::default().fg(theme.text_dim)
        } else {
            Style::default().fg(c)
        }
    }
}

/// Maintain a pool of particles.
pub fn step_particles(
    particles: &mut Vec<Particle>,
    width: u16,
    height: u16,
    tick: u64,
    max: usize,
) {
    for p in particles.iter_mut() {
        p.tick();
    }
    particles.retain(|p| {
        p.alive() && p.x >= 0.0 && p.y >= 0.0 && p.x < width as f32 && p.y < height as f32 + 2.0
    });
    while particles.len() < max && width > 0 {
        particles.push(Particle::spawn(
            width,
            height.max(1),
            tick.wrapping_add(particles.len() as u64),
        ));
        // Spawn at most a few per frame.
        if particles.len() % 3 == 0 {
            break;
        }
    }
}

/// Aurora gradient line shifting with tick.
pub fn aurora_line(width: u16, tick: u64, theme: &Theme) -> Line<'static> {
    if width == 0 {
        return Line::from("");
    }
    let colors = [
        theme.accent,
        theme.accent2,
        theme.accent3,
        theme.accent,
        theme.info,
    ];
    let mut spans = Vec::with_capacity(width as usize);
    let phase = (tick / 2) as usize;
    for i in 0..width as usize {
        let idx = (i / 3 + phase) % colors.len();
        let next = (idx + 1) % colors.len();
        // Alternate glyphs for shimmer.
        let ch = if (i + phase) % 7 == 0 {
            '✦'
        } else if (i + phase) % 5 == 0 {
            '·'
        } else {
            '─'
        };
        let c = if (i + phase) % 2 == 0 {
            colors[idx]
        } else {
            colors[next]
        };
        spans.push(Span::styled(ch.to_string(), Style::default().fg(c)));
    }
    Line::from(spans)
}

/// Splash state during boot animation.
#[derive(Clone, Debug)]
pub struct SplashState {
    pub frame: u64,
    pub max_frames: u64,
    pub particles: Vec<Particle>,
}

impl SplashState {
    pub fn new() -> Self {
        Self {
            frame: 0,
            max_frames: 28, // ~1.4s at 50ms
            particles: Vec::new(),
        }
    }

    pub fn done(&self) -> bool {
        self.frame >= self.max_frames
    }

    pub fn advance(&mut self) {
        self.frame += 1;
    }
}

/// ASCII claw logo frames (reveal).
pub fn claw_logo(frame: u64) -> Vec<&'static str> {
    let full = [
        r"        ▄▄▄▄▄▄▄▄▄          ",
        r"     ▄██▀▀░░░░░▀▀██▄       ",
        r"   ▄█▀░▄▄█████▄▄░░▀█▄      ",
        r"  █▀░███  CLAW  ███░▀█     ",
        r"  █░██▀  ▀███▀  ▀██░█      ",
        r"  ▀█░█  ╔═════╗  █░█▀      ",
        r"   ▀█▄  ║ RUST║  ▄█▀       ",
        r"     ▀██╗═════╔██▀         ",
        r"        ▀▀███▀▀            ",
    ];
    let reveal = ((frame as usize * full.len()) / 12).min(full.len());
    if frame < 12 {
        full[..reveal].to_vec()
    } else {
        full.to_vec()
    }
}

pub fn splash_subtitle(frame: u64, theme_name: &str) -> String {
    if frame < 14 {
        String::new()
    } else if frame < 20 {
        "coding agent harness".to_string()
    } else {
        format!("theme · {}  ·  press any key", theme_name)
    }
}

/// Pulse alpha-like border color oscillation.
pub fn pulse_border(tick: u64, theme: &Theme) -> Color {
    if (tick / 8) % 2 == 0 {
        theme.border_focus
    } else {
        theme.accent2
    }
}

/// Placeholder cycling text when input empty.
pub fn placeholder(tick: u64) -> &'static str {
    const PHRASES: &[&str] = &[
        "ask the claw…",
        "type / for commands",
        "Ctrl+P · command palette",
        "build · plan · explore",
    ];
    PHRASES[((tick / 40) as usize) % PHRASES.len()]
}

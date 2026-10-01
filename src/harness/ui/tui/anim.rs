//! Animation primitives: spinners, particles, splash, aurora.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use super::theme::Theme;

/// Braille spinner frames.
pub const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Dense circular braille spinner used for every *active* state (streaming,
/// working, tools running). Heavier than `SPINNER` so activity reads as a
/// spinning orb. All glyphs are braille (U+28xx, width 1), so they never
/// disturb layout the way a wide emoji would.
pub const THINK_SPIN: &[&str] = &["⣼", "⣹", "⢻", "⠿", "⡟", "⣏", "⣧", "⣶"];

/// Claw-ish alternate spinner.
// Kept as part of the animation API (alternate spinner style); not currently
// wired into the UI, but intentionally public for future use.
#[allow(dead_code)]
pub const CLAW_SPIN: &[&str] = &["ᕙ", "ᕗ", "ᕕ", "ᕙ", "ᕗ", "ᕕ"];

/// Streaming / cursor blink glyphs.
///
/// The TUI hides the terminal's own cursor (see `runner::run_tui`) and draws
/// this glyph instead, so its shape is fully under our control and does not
/// fight with the terminal's cursor style. U+258F (left one eighth block) is
/// the thinnest of the block family and stays left-aligned in the cell, so
/// the layout is untouched (width 1, same as before).
pub const CURSOR_ON: &str = "▏";
pub const CURSOR_OFF: &str = " ";

pub fn spinner_frame(tick: u64) -> &'static str {
    SPINNER[(tick as usize / 2) % SPINNER.len()]
}

/// Frame of the dense circular braille orb used for active states.
pub fn think_frame(tick: u64) -> &'static str {
    THINK_SPIN[(tick as usize / 2) % THINK_SPIN.len()]
}

/// Bar-glyph "audio wave" frames, from silent to loud. Used by the recording
/// indicator to fake an animated waveform (we do not expose real mic
/// amplitude to the UI, so the wave is a traveling sine).
pub const WAVE_BARS: &[&str] = &["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// One column of the fake audio wave for `tick` at horizontal position `i`.
///
/// A sine phase travels right as `tick` advances; each column's height is a
/// bar glyph picked from [`WAVE_BARS`]. All glyphs are width 1, so layout is
/// untouched.
pub fn wave_column(tick: u64, i: usize) -> &'static str {
    use std::f64::consts::TAU;
    const SPEED: f64 = 0.55; // phase shift per tick
    const PERIOD: f64 = 4.5; // columns per full oscillation
    let t = tick as f64 * SPEED + i as f64 * TAU / PERIOD;
    // Sum two sines for a less mechanical look, normalized to 0..1.
    let v = ((t.sin() + 1.0) * 0.5 * 0.7 + (t * 2.3).sin().abs() * 0.3).clamp(0.0, 1.0);
    let idx = (v * (WAVE_BARS.len() - 1) as f64).round() as usize;
    WAVE_BARS[idx]
}

/// A short animated waveform (6 columns) for the recording indicator.
///
/// `level` (0..=1000, from the recorder's live RMS) modulates the wave
/// height; when it is `None` or near silence the traveling sine alone
/// drives the animation.
pub fn wave_frame(tick: u64, level: Option<u32>) -> String {
    let boost = level.unwrap_or(0) as f64 / 1000.0;
    (0..6)
        .map(|i| {
            let base = wave_column(tick, i);
            let base_idx = WAVE_BARS.iter().position(|b| *b == base).unwrap_or(0);
            // Real mic level scales the height; keep a small floor so the
            // wave stays visible during silence.
            let scaled = base_idx as f64 * (0.25 + 0.75 * boost);
            WAVE_BARS[(scaled.round() as usize).min(WAVE_BARS.len() - 1)]
        })
        .collect()
}

/// Number of draw ticks a freshly streamed reasoning line takes to settle from
/// its "hot" accent color to the resting dim color (~0.4 s at 30 fps).
pub const FADE_TICKS: u64 = 12;

/// Linear blend between two RGB colors. `t` is clamped to `0.0..=1.0`.
/// Non-RGB colors fall back to `to` (the resting color).
pub fn blend(
    from: ratatui::style::Color,
    to: ratatui::style::Color,
    t: f32,
) -> ratatui::style::Color {
    use ratatui::style::Color;
    let t = t.clamp(0.0, 1.0);
    match (from, to) {
        (Color::Rgb(fr, fg, fb), Color::Rgb(tr, tg, tb)) => {
            let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
            Color::Rgb(mix(fr, tr), mix(fg, tg), mix(fb, tb))
        }
        _ => to,
    }
}

/// Color for a reasoning line born at `born_tick`, observed at `tick`.
///
/// `born_tick == 0` means the line is already at rest (history/tests), so it
/// returns `rest` directly. Otherwise it fades from `hot` to `rest` over
/// [`FADE_TICKS`], after which it is stable and costs nothing to render.
pub fn reasoning_fade(
    born_tick: u64,
    tick: u64,
    hot: ratatui::style::Color,
    rest: ratatui::style::Color,
) -> ratatui::style::Color {
    if born_tick == 0 {
        return rest;
    }
    let age = tick.saturating_sub(born_tick);
    if age >= FADE_TICKS {
        return rest;
    }
    blend(hot, rest, age as f32 / FADE_TICKS as f32)
}

/// Alternate claw spinner frame. Part of the animation API; not currently
/// wired into the UI but intentionally public.
#[allow(dead_code)]
pub fn claw_frame(tick: u64) -> &'static str {
    CLAW_SPIN[(tick as usize / 3) % CLAW_SPIN.len()]
}

pub fn cursor_glyph(tick: u64) -> &'static str {
    if (tick / 6).is_multiple_of(2) {
        CURSOR_ON
    } else {
        CURSOR_OFF
    }
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
        if particles.len().is_multiple_of(3) {
            break;
        }
    }
}

/// Aurora gradient line shifting with tick. Part of the animation API; not
/// currently wired into the UI but intentionally public.
#[allow(dead_code)]
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
        let ch = if (i + phase).is_multiple_of(7) {
            '✦'
        } else if (i + phase).is_multiple_of(5) {
            '·'
        } else {
            '─'
        };
        let c = if (i + phase).is_multiple_of(2) {
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

/// ASCII "RUSTCLAW" wordmark frames (reveal).
pub fn claw_logo(frame: u64) -> Vec<&'static str> {
    let full = [
        r"██████╗ ██╗   ██╗███████╗████████╗ ██████╗ ██╗      █████╗ ██╗    ██╗",
        r"██╔══██╗██║   ██║██╔════╝╚══██╔══╝██╔════╝ ██║     ██╔══██╗██║    ██║",
        r"██████╔╝██║   ██║███████╗   ██║   ██║      ██║     ███████║██║ █╗ ██║",
        r"██╔══██╗██║   ██║╚════██║   ██║   ██║      ██║     ██╔══██║██║███╗██║",
        r"██║  ██║╚██████╔╝███████║   ██║   ╚██████╗ ███████╗██║  ██║╚███╔███╔╝",
        r"╚═╝  ╚═╝ ╚═════╝ ╚══════╝   ╚═╝    ╚═════╝ ╚══════╝╚═╝  ╚═╝ ╚══╝╚══╝ ",
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
    if (tick / 8).is_multiple_of(2) {
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
        "build · reason · chat-free",
    ];
    PHRASES[((tick / 40) as usize) % PHRASES.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claw_logo_rows_are_aligned() {
        // Every row of the wordmark must have the same display width, otherwise
        // the ASCII art shears. Guards against future edits that drop a space.
        let logo = claw_logo(100);
        assert!(!logo.is_empty());
        let widths: Vec<usize> = logo.iter().map(|r| r.chars().count()).collect();
        let first = widths[0];
        assert!(
            widths.iter().all(|w| *w == first),
            "logo rows must share one width, got {widths:?}"
        );
    }

    #[test]
    fn claw_logo_reveals_progressively() {
        // Early frames show fewer rows; by frame 12 the full art is visible.
        assert!(claw_logo(0).len() < claw_logo(100).len());
        assert_eq!(claw_logo(12).len(), claw_logo(100).len());
    }

    #[test]
    fn think_frame_cycles_through_all_orb_glyphs() {
        // The orb must actually animate: consecutive ticks yield different
        // frames, and every glyph in the set is reachable.
        let seen: std::collections::HashSet<&str> = (0..(THINK_SPIN.len() as u64 * 2))
            .map(think_frame)
            .collect();
        assert_eq!(seen.len(), THINK_SPIN.len());
        assert_ne!(think_frame(0), think_frame(2));
    }

    #[test]
    fn wave_frame_animates_and_is_width_one_per_column() {
        // Consecutive ticks produce different waves; every glyph is width 1
        // (bar blocks) so the status line layout is never disturbed.
        assert_ne!(wave_frame(0, None), wave_frame(1, None));
        assert_eq!(wave_frame(0, None).chars().count(), 6);
        let allowed: std::collections::HashSet<char> =
            WAVE_BARS.iter().flat_map(|b| b.chars()).collect();
        assert!(wave_frame(7, None).chars().all(|c| allowed.contains(&c)));
        // The wave actually moves through the full height range over time.
        let seen: std::collections::HashSet<&str> = (0..64)
            .flat_map(|t| (0..6).map(move |i| wave_column(t, i)))
            .collect();
        assert_eq!(seen.len(), WAVE_BARS.len());
    }

    #[test]
    fn reasoning_fade_is_stable_when_born_tick_is_zero() {
        // History/tests stamp `0`; those lines must render at rest, never hot.
        let hot = ratatui::style::Color::Rgb(255, 0, 0);
        let rest = ratatui::style::Color::Rgb(0, 0, 255);
        assert_eq!(reasoning_fade(0, 0, hot, rest), rest);
        assert_eq!(reasoning_fade(0, 999, hot, rest), rest);
    }

    #[test]
    fn reasoning_fade_interpolates_then_settles() {
        let hot = ratatui::style::Color::Rgb(255, 0, 0);
        let rest = ratatui::style::Color::Rgb(0, 0, 255);
        // At birth the line is fully hot.
        assert_eq!(reasoning_fade(100, 100, hot, rest), hot);
        // Midway it is a blend, distinct from both endpoints.
        let mid = reasoning_fade(100, 100 + FADE_TICKS / 2, hot, rest);
        assert_ne!(mid, hot);
        assert_ne!(mid, rest);
        // Once the fade window elapses it is stable at rest.
        assert_eq!(reasoning_fade(100, 100 + FADE_TICKS, hot, rest), rest);
        assert_eq!(reasoning_fade(100, 100 + FADE_TICKS * 10, hot, rest), rest);
    }

    #[test]
    fn blend_clamps_and_falls_back_for_non_rgb() {
        use ratatui::style::Color;
        let a = Color::Rgb(0, 0, 0);
        let b = Color::Rgb(100, 200, 50);
        assert_eq!(blend(a, b, 0.0), a);
        assert_eq!(blend(a, b, 1.0), b);
        assert_eq!(blend(a, b, 2.0), b); // clamped
                                         // Non-RGB endpoints fall back to the resting color.
        assert_eq!(blend(Color::Red, b, 0.5), b);
    }
}

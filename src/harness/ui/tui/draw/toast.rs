//! Floating toast notifications (bottom-right, over the transcript).
//!
//! Transient confirmations — "copied 42 chars", "image attached", "code block
//! saved" — used to be pushed into `status_msg`, which is a single slot shared
//! with turn state ("running…", "cancelling…") and therefore easy to overwrite
//! before the user ever reads it. A toast is an independent, ephemeral slot
//! that fades out on its own and never clobbers turn status.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::harness::ui::tui::anim::{blend, FADE_TICKS};
use crate::harness::ui::tui::app::App;
use crate::harness::ui::tui::theme::Theme;

/// Total lifetime of a toast in draw ticks: ~2.5 s at 30 fps, of which the
/// last `FADE_TICKS` are spent fading out.
pub const TOAST_TICKS: u64 = 75;

/// Semantic flavor of a toast, which selects its icon and accent color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

impl ToastKind {
    fn icon(self) -> &'static str {
        match self {
            ToastKind::Info => "ℹ",
            ToastKind::Success => "✓",
            ToastKind::Error => "✗",
        }
    }

    fn accent(self, t: &Theme) -> ratatui::style::Color {
        match self {
            ToastKind::Info => t.accent,
            ToastKind::Success => t.success,
            ToastKind::Error => t.error,
        }
    }
}

/// A single live toast. Only one is shown at a time; a new one replaces it.
#[derive(Clone, Debug)]
pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    /// Draw tick when the toast was raised (0 = never animated).
    pub born_tick: u64,
}

impl Toast {
    pub fn new(text: impl Into<String>, kind: ToastKind, born_tick: u64) -> Self {
        Self {
            text: text.into(),
            kind,
            born_tick,
        }
    }

    /// True once the toast has outlived `TOAST_TICKS` and should be dropped.
    pub fn expired(&self, tick: u64) -> bool {
        self.born_tick != 0 && tick.saturating_sub(self.born_tick) >= TOAST_TICKS
    }

    /// Opacity `1.0 → 0.0` over the trailing `FADE_TICKS` of the lifetime.
    fn opacity(&self, tick: u64) -> f32 {
        if self.born_tick == 0 {
            return 1.0;
        }
        let age = tick.saturating_sub(self.born_tick);
        if age + FADE_TICKS <= TOAST_TICKS {
            return 1.0;
        }
        let over = (age + FADE_TICKS).saturating_sub(TOAST_TICKS);
        1.0 - (over as f32 / FADE_TICKS as f32).clamp(0.0, 1.0)
    }

    /// Width of the rendered box (border + padding + icon + text), clamped to
    /// `max_w`. Returned width is always at least 4 so the box stays legible.
    fn box_width(&self, max_w: u16) -> u16 {
        let inner = Span::raw(self.text.as_str()).width() as u16 + 4; // "✗ " + padding
        inner.saturating_add(2).clamp(4, max_w.max(4))
    }
}

/// Draws the active toast (if any) anchored to the bottom-right of `area`.
///
/// Called last in the draw pass so the toast floats over the transcript, but
/// before modals so a permission prompt still takes visual precedence.
pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let Some(toast) = app.toast.as_ref() else {
        return;
    };
    if area.width < 8 || area.height < 3 {
        return;
    }

    let opacity = toast.opacity(app.tick);
    if opacity <= 0.0 {
        return;
    }

    let accent = toast.kind.accent(&app.theme);
    let dim = app.theme.text_dim;
    // Fade the accent toward the background as the toast expires.
    let faded = blend(accent, dim, 1.0 - opacity);
    let border = blend(accent, dim, (1.0 - opacity) * 0.6);

    let w = toast.box_width(area.width.saturating_sub(4));
    let h = 3u16;
    let x = area.x + area.width.saturating_sub(w + 2);
    let y = area.y + area.height.saturating_sub(h + 1);
    let box_area = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    let inner_w = w.saturating_sub(2) as usize;
    // "✗ text" — icon (width 1) + space + text, truncated to the inner width.
    let icon = toast.kind.icon();
    let icon_w = Span::raw(icon).width();
    let avail = inner_w.saturating_sub(icon_w + 1);
    let mut label = toast.text.clone();
    if Span::raw(label.as_str()).width() > avail {
        label = truncate_to_width(&label, avail);
    }
    let used = icon_w + 1 + Span::raw(label.as_str()).width();
    let pad = inner_w.saturating_sub(used);

    let line = Line::from(vec![
        Span::styled(
            format!("{icon} "),
            Style::default().fg(faded).add_modifier(Modifier::BOLD),
        ),
        Span::styled(label, Style::default().fg(faded)),
        Span::raw(" ".repeat(pad)),
    ]);

    let block = ratatui::widgets::Block::bordered()
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .style(Style::default().bg(app.theme.status_bg));

    frame.render_widget(
        ratatui::widgets::Paragraph::new(line).block(block),
        box_area,
    );
}

/// Truncates `s` so its display width is at most `max`, appending "…" when cut.
fn truncate_to_width(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = Span::raw(ch.to_string()).width();
        if w + cw > max.saturating_sub(1) {
            out.push('…');
            return out;
        }
        out.push(ch);
        w += cw;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_after_lifetime() {
        let t = Toast::new("hi", ToastKind::Info, 10);
        assert!(!t.expired(10));
        assert!(!t.expired(10 + TOAST_TICKS - 1));
        assert!(t.expired(10 + TOAST_TICKS));
    }

    #[test]
    fn static_toast_never_expires() {
        let t = Toast::new("hi", ToastKind::Info, 0);
        assert!(!t.expired(u64::MAX));
    }

    #[test]
    fn opacity_holds_then_fades() {
        let t = Toast::new("hi", ToastKind::Info, 100);
        assert_eq!(t.opacity(100), 1.0);
        assert_eq!(t.opacity(100 + TOAST_TICKS - FADE_TICKS - 1), 1.0);
        // Midway through the fade window.
        let mid = t.opacity(100 + TOAST_TICKS - FADE_TICKS / 2);
        assert!(
            mid > 0.0 && mid < 1.0,
            "expected partial opacity, got {mid}"
        );
        assert_eq!(t.opacity(100 + TOAST_TICKS), 0.0);
    }

    #[test]
    fn box_width_grows_with_text_but_is_clamped() {
        let short = Toast::new("ok", ToastKind::Info, 1);
        let long = Toast::new("x".repeat(200), ToastKind::Info, 1);
        assert!(short.box_width(80) < long.box_width(80));
        assert_eq!(long.box_width(30), 30);
    }

    #[test]
    fn truncate_respects_width() {
        assert_eq!(truncate_to_width("hello", 10), "hello");
        assert_eq!(truncate_to_width("hello world", 6), "hello…");
        assert_eq!(truncate_to_width("hello", 0), "");
    }
}

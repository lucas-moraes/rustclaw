# Phase 4: Dynamic Locale Switching

## Status: ✅ Complete

## Modified Files

- `src/i18n/mod.rs` - Added runtime locale switching
- `src/cli.rs` - Added `/locale` command

## Implementation Details

### Runtime Locale Storage
Added `static RUNTIME_LOCALE: OnceLock<Locale>` to store the locale at runtime.

### New Locale Methods

```rust
impl Locale {
    pub fn current() -> Locale {
        RUNTIME_LOCALE.get().copied().unwrap_or_else(Locale::from_env)
    }

    pub fn set(locale: Locale) {
        let _ = RUNTIME_LOCALE.set(locale);
    }

    pub fn from_string(s: &str) -> Option<Locale> {
        match s.to_lowercase().as_str() {
            "en" | "english" => Some(Locale::En),
            "pt_br" | "pt" | "portuguese" | "br" => Some(Locale::PtBr),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::PtBr => "pt_br",
        }
    }
}
```

### Updated `t()` Function
Changed from `Locale::from_env()` to `Locale::current()` to use runtime locale.

### CLI Commands Added

**`/locale`** - Shows current locale:
```
⬡  Locale

  Current locale: pt_br
  Available: en, pt_br

  Use: /locale <lang>
```

**`/locale <lang>`** - Changes locale:
```
/locale en    → Changes to English
/locale pt_br → Changes to Portuguese
```

### Priority
Lower priority than features 1-5, but i18n infrastructure is now complete.

## Verification

- ✅ `cargo clippy --quiet` - 0 warnings
- ✅ `cargo test` - 114 tests pass

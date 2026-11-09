//! Conservative recognition of shell commands that actually perform verification.
//!
//! This is deliberately not a general-purpose shell parser. Unsupported shell syntax
//! is rejected rather than guessed at, so quoted mentions and successful wrappers
//! cannot stand in for a verification command.

/// Return the complete, trimmed command as a label when it performs verification.
///
/// Only a single verification command, optionally preceded by literal environment
/// assignments, `env`, and one `cd path &&`, is accepted. Pipelines, redirections,
/// substitutions, background execution, and commands after verification are rejected.
pub fn verification_command(command: &str) -> Option<String> {
    let command = command.trim();
    let tokens = tokenize(command)?;
    let mut words = Vec::new();
    let mut changed_directory = false;

    for token in tokens {
        match token {
            Token::Word(word) => words.push(word),
            Token::And if !changed_directory && is_cd(&words) => {
                changed_directory = true;
                words.clear();
            }
            Token::And => return None,
        }
    }

    let mut words = words.as_slice();
    while words.first().is_some_and(|word| word.assignment) {
        if unsafe_environment(&words[0].value) {
            return None;
        }
        words = &words[1..];
    }
    if words.first().is_some_and(|word| word.value == "env") {
        words = &words[1..];
        if words.first().is_some_and(|word| word.value == "--") {
            words = &words[1..];
        }
        while words.first().is_some_and(|word| is_assignment(&word.value)) {
            if unsafe_environment(&words[0].value) {
                return None;
            }
            words = &words[1..];
        }
    }

    let values: Vec<&str> = words.iter().map(|word| word.value.as_str()).collect();
    is_verification(&values).then(|| command.to_owned())
}

#[derive(Debug)]
struct Word {
    value: String,
    assignment: bool,
}

#[derive(Debug)]
enum Token {
    Word(Word),
    And,
}

fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn unsafe_environment(assignment: &str) -> bool {
    let (name, value) = assignment.split_once('=').unwrap_or_default();
    let name = name.to_ascii_lowercase();
    // These variables inject extra arguments or replace the verifier. Ordinary
    // build settings such as CARGO_TARGET_DIR and CARGO_TERM_COLOR remain valid.
    !value.is_empty()
        && (matches!(
            name.as_str(),
            "pytest_addopts" | "goflags" | "cargo" | "rustc" | "rustdoc" | "rustfmt"
        ) || name.starts_with("npm_config_")
            || name.starts_with("cargo_alias_")
            || name.starts_with("cargo_target_") && name.ends_with("_runner")
            || name.starts_with("rustc_")
            || name == "cargo_build_rustc"
            || name == "cargo_build_rustc_wrapper"
            || name == "cargo_build_rustc_workspace_wrapper")
}

fn is_cd(words: &[Word]) -> bool {
    let path = match words {
        [command, path] if command.value == "cd" => &path.value,
        [command, separator, path] if command.value == "cd" && separator.value == "--" => {
            &path.value
        }
        _ => return false,
    };
    !path.is_empty() && !path.starts_with('-')
}

fn tokenize(command: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut chars = command.char_indices().peekable();
    let mut start = None;
    let mut value = String::new();
    let mut quote = None;

    while let Some((index, ch)) = chars.next() {
        // Even literal newlines are outside the supported single-line grammar.
        if matches!(ch, '\n' | '\r' | '\0') {
            return None;
        }
        if let Some(delimiter) = quote {
            if ch == delimiter {
                quote = None;
            } else if delimiter == '"' && matches!(ch, '$' | '`') {
                return None;
            } else if delimiter == '"' && ch == '\\' {
                let (_, escaped) = chars.next()?;
                if matches!(escaped, '\n' | '\r' | '\0') {
                    return None;
                }
                if !matches!(escaped, '$' | '`' | '"' | '\\') {
                    value.push('\\');
                }
                value.push(escaped);
            } else {
                value.push(ch);
            }
            continue;
        }

        match ch {
            ' ' | '\t' => finish_word(command, &mut tokens, &mut start, &mut value, index),
            '\'' | '"' => {
                start.get_or_insert(index);
                quote = Some(ch);
            }
            '\\' => {
                start.get_or_insert(index);
                let (_, escaped) = chars.next()?;
                if matches!(escaped, '\n' | '\r' | '\0') {
                    return None;
                }
                value.push(escaped);
            }
            '&' if chars.peek().is_some_and(|(_, ch)| *ch == '&') => {
                finish_word(command, &mut tokens, &mut start, &mut value, index);
                chars.next();
                tokens.push(Token::And);
            }
            // Reject shell operators, comments, expansion, and globbing. Their
            // literal equivalents inside quotes are safe and remain ordinary words.
            '|' | ';' | '&' | '<' | '>' | '(' | ')' | '{' | '}' | '$' | '`' | '#' | '*' | '?'
            | '[' | ']' | '~' => return None,
            _ => {
                start.get_or_insert(index);
                value.push(ch);
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    finish_word(command, &mut tokens, &mut start, &mut value, command.len());
    Some(tokens)
}

fn finish_word(
    command: &str,
    tokens: &mut Vec<Token>,
    start: &mut Option<usize>,
    value: &mut String,
    end: usize,
) {
    if let Some(start) = start.take() {
        tokens.push(Token::Word(Word {
            value: std::mem::take(value),
            // Assignment names must be unquoted in the original shell source.
            assignment: is_assignment(&command[start..end]),
        }));
    }
}

fn is_verification(words: &[&str]) -> bool {
    // These flags can return success without running the requested verification.
    if words.iter().any(|word| {
        matches!(
            word.split('=').next().unwrap_or_default(),
            "--help"
                | "-h"
                | "--version"
                | "-V"
                | "--dry-run"
                | "--usage"
                | "--ignore-scripts"
                | "--if-present"
        ) || (word.starts_with('-') && !word.starts_with("--") && word.contains('h'))
    }) {
        return false;
    }
    match words {
        ["cargo", rest @ ..] => {
            // Cargo configuration can replace compilers or test runners.
            if rest
                .iter()
                .any(|arg| arg.split('=').next() == Some("--config"))
            {
                return false;
            }
            let rest = if rest.first().is_some_and(|word| word.starts_with('+')) {
                if rest[0].len() == 1 {
                    return false;
                }
                &rest[1..]
            } else {
                rest
            };
            match rest {
                ["test", args @ ..] => !args.contains(&"--list"),
                ["check" | "build" | "clippy", ..] => true,
                ["fmt", args @ ..] => is_fmt_check(args),
                _ => false,
            }
        }
        ["npm" | "pnpm" | "yarn", "test", ..]
        | ["npm", "run", "test" | "lint" | "typecheck" | "build", ..] => true,
        ["pytest", args @ ..] | ["python" | "python3", "-m", "pytest", args @ ..] => {
            !args.iter().any(|arg| {
                matches!(
                    arg.split('=').next().unwrap_or_default(),
                    "--collect-only"
                        | "--co"
                        | "--fixtures"
                        | "--fixtures-per-test"
                        | "--funcargs"
                        | "--markers"
                        | "--trace-config"
                )
            })
        }
        ["go", "test" | "vet", args @ ..] => !args.iter().any(|arg| {
            matches!(
                arg.split('=').next().unwrap_or_default(),
                "-n" | "-list" | "-exec" | "-toolexec" | "-vettool"
            )
        }),
        ["git", "diff", args @ ..] => is_diff_check(args),
        _ => false,
    }
}

fn is_fmt_check(mut args: &[&str]) -> bool {
    let mut check = false;
    while let Some((arg, remaining)) = args.split_first() {
        args = remaining;
        match *arg {
            "--check" => check = true,
            "--" | "--all" | "--verbose" | "-v" | "--quiet" | "-q" => {}
            "--manifest-path" | "--package" | "-p" => {
                let Some((value, remaining)) = args.split_first() else {
                    return false;
                };
                if value.is_empty() || value.starts_with('-') {
                    return false;
                }
                args = remaining;
            }
            arg if arg.starts_with("--manifest-path=") || arg.starts_with("--package=") => {}
            _ => return false,
        }
    }
    check
}

fn is_diff_check(args: &[&str]) -> bool {
    let mut check = false;
    for arg in args {
        match *arg {
            "--" => break,
            "--check" => check = true,
            "--cached" | "--staged" | "--no-ext-diff" | "--no-textconv" | "--no-color"
            | "--no-renames" | "--no-index" | "-R" => {}
            arg if !arg.starts_with('-') => {}
            _ => return false,
        }
    }
    check
}

#[cfg(test)]
mod tests {
    use super::verification_command;

    #[test]
    fn accepts_real_verification_commands() {
        for command in [
            "cargo test",
            "cargo +1.90.0 test --workspace",
            "cargo +nightly clippy --all-targets -- -D warnings",
            "cargo check --all-features",
            "cargo build --release",
            "cargo test --no-run",
            "cargo fmt --check",
            "cargo fmt --all -- --check",
            "cargo fmt --package example --check",
            "npm test",
            "pnpm test -- --runInBand",
            "yarn test",
            "npm run test",
            "npm run lint",
            "npm run typecheck",
            "npm run build -- --mode production",
            "pytest -q tests",
            "python -m pytest",
            "python3 -m pytest tests/test_example.py",
            "go test ./...",
            "go vet ./...",
            "git diff --check",
            "git diff --cached --check",
            "git diff --check -- docs/README.md",
        ] {
            assert_eq!(verification_command(command).as_deref(), Some(command));
        }
    }

    #[test]
    fn accepts_literal_prefixes_and_preserves_the_complete_label() {
        for command in [
            "cd path&&cargo test",
            "cd 'path with spaces' && cargo +1.90.0 test",
            "cd -- path && cargo check",
            "CARGO_TERM_COLOR=never cargo test",
            "CARGO_TARGET_DIR='target dir' CI=1 cargo check",
            "env CI=1 cargo test",
            "env 'CI=1' cargo test",
            "CI=1 env -- CARGO_TERM_COLOR=never cargo test",
            "cd path && env CI=1 cargo fmt --check",
            "'cargo' \"test\" 'literal | argument'",
            "cargo test 'literal $(not executed)'",
            "cargo test \"literal \\$value\"",
        ] {
            assert_eq!(verification_command(command).as_deref(), Some(command));
        }
        assert_eq!(
            verification_command("  cargo test  ").as_deref(),
            Some("cargo test")
        );
    }

    #[test]
    fn rejects_masked_status_and_additional_commands() {
        for command in [
            "cargo test|tail",
            "cargo test | tail -1",
            "set -o pipefail && cargo test | tail",
            "cargo test;true",
            "cargo test&&echo done",
            "cargo test||true",
            "cargo test &",
            "cargo test & echo done",
            "cargo test > /dev/null",
            "cargo test 2>&1",
            "cargo test < input",
            "cargo test\necho done",
            "cargo test\rtrue",
            "cargo test # comment",
            "cargo fmt # --check",
            "cargo test -- 'literal ; argument' && echo done",
            "cd path && cargo test && true",
            "cd path && cd nested && cargo test",
            "true && cargo test",
            "&& cargo test",
            "cargo test &&",
            "(cargo test)",
            "{ cargo test; }",
        ] {
            assert_eq!(verification_command(command), None, "{command}");
        }
    }

    #[test]
    fn rejects_mentions_dynamic_syntax_and_malformed_quotes() {
        for command in [
            "",
            "   ",
            "echo cargo test",
            "echo 'cargo test'",
            "printf '%s' 'cargo test'",
            "'cargo test'",
            "bash -c 'cargo test'",
            "sh -c \"cargo test\"",
            "ls -la",
            "cargo tester",
            "cargo",
            "cargo + test",
            "cargo test $(echo injected)",
            "cargo test `echo injected`",
            "cargo test \"$(echo injected)\"",
            "cargo test \"$FLAGS\"",
            "CARGO_TARGET_DIR=$(pwd) cargo test",
            "cargo test *.rs",
            "cargo test \\\n--help",
            "cargo test 'unterminated",
            "cargo test \"unterminated",
            "cargo test \\",
            "cargo test\0",
            "\"CI=1\" cargo test",
            "'CI'=1 cargo test",
            "CI\\=1 cargo test",
            "env -S 'echo cargo test'",
            "cd && cargo test",
            "cd - && cargo test",
        ] {
            assert_eq!(verification_command(command), None, "{command}");
        }
    }

    #[test]
    fn rejects_non_verifying_modes_and_ambiguous_check_flags() {
        for command in [
            "cargo fmt",
            "cargo fmt --all",
            "cargo fmt --manifest-path --check",
            "cargo fmt --package=--check",
            "cargo fmt --check --config-path arbitrary",
            "cargo test --help",
            "cargo test -qh",
            "cargo test --config 'target.x86_64-unknown-linux-gnu.runner=echo'",
            "cargo test -- --help",
            "cargo test -- --list",
            "cargo check --version",
            "npm run arbitrary",
            "npm run test --if-present",
            "npm test --ignore-scripts=true",
            "npm test --dry-run",
            "npm test --usage",
            "npm test -vh",
            "pytest --collect-only",
            "pytest --fixtures",
            "pytest --markers",
            "PYTEST_ADDOPTS=--collect-only pytest",
            "env GOFLAGS=-n go test",
            "npm_config_usage=true npm test",
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER=echo cargo test",
            "RUSTC_WRAPPER=echo cargo check",
            "python -c 'import pytest'",
            "go test -n ./...",
            "go test -list=. ./...",
            "go test -exec echo ./...",
            "go vet -vettool echo ./...",
            "git diff",
            "git diff -- --check",
            "git diff --check --no-check",
            "git diff --check --quiet",
            "git diff --check --output=hidden",
            "git diff --src-prefix --check",
        ] {
            assert_eq!(verification_command(command), None, "{command}");
        }
    }
}

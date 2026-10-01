//! Permission commands: /permissions, /allow-all-permissions.
//!
//! Extracted from `mod.rs`.

use crate::harness::runtime::SessionRuntime;
use anyhow::Result;

/// Human-readable label for a permission rule.
fn rule_label(rule: crate::harness::permission::Rule) -> &'static str {
    match rule {
        crate::harness::permission::Rule::Allow => "allow",
        crate::harness::permission::Rule::Ask => "ask",
        crate::harness::permission::Rule::Deny => "deny",
    }
}

pub(crate) fn handle_permissions_cmd(
    runtime: &mut SessionRuntime,
    cmd: &str,
    arg: &str,
    out: &mut Vec<String>,
) -> Result<()> {
    match cmd {
        "/permissions" => {
            let mut parts = arg.split_whitespace();
            let sub = parts.next().unwrap_or("");
            match sub {
                "" | "list" => {
                    let rules = runtime.permission_rules();
                    if rules.is_empty() {
                        out.push("no per-tool permission rules (defaults apply)".to_string());
                    } else {
                        out.push(format!("permission rules ({}):", rules.len()));
                        for (tool, rule) in rules {
                            out.push(format!("  {} = {}", tool, rule_label(rule)));
                        }
                    }
                    out.push(
                        "usage: /permissions set <tool> <allow|ask|deny> · rm <tool>".to_string(),
                    );
                }
                "set" => {
                    let tool = parts.next().unwrap_or("");
                    let rule = parts.next().unwrap_or("");
                    if tool.is_empty() || rule.is_empty() {
                        out.push("usage: /permissions set <tool> <allow|ask|deny>".to_string());
                    } else {
                        let parsed = match rule.to_lowercase().as_str() {
                            "allow" | "a" => Some(crate::harness::permission::Rule::Allow),
                            "ask" => Some(crate::harness::permission::Rule::Ask),
                            "deny" | "d" => Some(crate::harness::permission::Rule::Deny),
                            _ => None,
                        };
                        match parsed {
                            Some(r) => match runtime.set_permission_rule(tool, r) {
                                Ok(()) => out.push(format!(
                                    "permission: {} = {} (saved to rustclaw.json)",
                                    tool,
                                    rule_label(r)
                                )),
                                Err(e) => out.push(format!("[error] {}", e)),
                            },
                            None => {
                                out.push(format!("unknown rule: {} (allow · ask · deny)", rule))
                            }
                        }
                    }
                }
                "rm" | "remove" => {
                    let tool = parts.next().unwrap_or("");
                    if tool.is_empty() {
                        out.push("usage: /permissions rm <tool>".to_string());
                    } else {
                        match runtime.remove_permission_rule(tool) {
                            Ok(true) => out.push(format!(
                                "permission rule for `{}` removed (falls back to default)",
                                tool
                            )),
                            Ok(false) => out.push(format!("no rule for tool `{}`", tool)),
                            Err(e) => out.push(format!("[error] {}", e)),
                        }
                    }
                }
                _ => out.push(format!("unknown subcommand: {} (list · set · rm)", sub)),
            }
        }
        "/allow-all-permissions" => match runtime.allow_all_permissions() {
            Ok(()) => {
                out.push(
                    "✅ all permissions granted: the harness may modify any file inside \
                         the project (saved to rustclaw.json)."
                        .to_string(),
                );
                out.push(
                    "Paths outside the project still require approval. Use \
                         `/permissions rm <tool>` to revoke a specific tool."
                        .to_string(),
                );
            }
            Err(e) => out.push(format!("[error] {}", e)),
        },
        // Invariant: the dispatcher in `ui/commands/mod.rs` only routes
        // `/permissions` and `/allow-all-permissions` here (line ~276), so no
        // other `cmd` can reach this match. If a new command is added to the
        // help but not routed, this fires loudly instead of silently no-oping.
        _ => unreachable!("handle_permissions_cmd called with unknown cmd: {}", cmd),
    }
    Ok(())
}

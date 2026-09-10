//! Provider and auth commands: /models, /model, /provider, /auth.
//!
//! Extracted from `mod.rs`.

use crate::harness::runtime::SessionRuntime;
use anyhow::Result;

pub(crate) async fn handle_provider_cmd(
    runtime: &mut SessionRuntime,
    cmd: &str,
    arg: &str,
    out: &mut Vec<String>,
) -> Result<()> {
    match cmd {
        "/models" => {
            use crate::harness::provider::catalog;
            use crate::harness::provider::user_store::UserProviders;
            let mut parts = arg.split_whitespace();
            let sub = parts.next().unwrap_or("");
            match sub {
                "" => {
                    out.push(format!(
                        "current: provider `{}` · model `{}`",
                        runtime.config.provider, runtime.config.model
                    ));
                    out.push(format!(
                        "providers: {} · usage: /models <provider> · /models add <provider> <model>",
                        catalog::provider_names().join(", ")
                    ));
                }
                "add" => {
                    let provider = parts.next().unwrap_or("");
                    let model = parts.next().unwrap_or("");
                    if provider.is_empty() || model.is_empty() {
                        out.push("usage: /models add <provider> <model>".to_string());
                    } else {
                        let mut store = UserProviders::load();
                        if store.add_model_anywhere(provider, model) {
                            match store.save() {
                                Ok(()) => out.push(format!(
                                    "model `{}` added to provider `{}`",
                                    model, provider
                                )),
                                Err(e) => out.push(format!("[error] {}", e)),
                            }
                        } else {
                            out.push(format!("unknown provider: `{}`", provider));
                        }
                    }
                }
                _ => {
                    let models = catalog::models_for(sub);
                    if models.is_empty() {
                        out.push(format!("unknown provider: {}", sub));
                    } else {
                        out.push(format!("models for {} ({}):", sub, models.len()));
                        for m in models {
                            out.push(format!("  {}", m));
                        }
                        out.push("switch with /provider <name> or /model <name>".to_string());
                    }
                }
            }
        }
        "/model" => {
            if arg.is_empty() {
                out.push(format!(
                    "current model: {} (provider {}) · usage: /model <name>",
                    runtime.config.model, runtime.config.provider
                ));
            } else {
                let provider = runtime.config.provider.clone();
                runtime.switch_model(&provider, arg)?;
                out.push(format!("model → {} ({})", arg, runtime.provider.name()));
                out.push("selection saved to config.json (global)".to_string());
                if !runtime.has_token_for(&provider) {
                    out.push(format!(
                        "no token for provider `{}` — use /auth {} to add one",
                        provider, provider
                    ));
                }
            }
        }
        "/provider" => {
            use crate::harness::provider::catalog;
            use crate::harness::provider::user_store::{UserProvider, UserProviders};
            let mut parts = arg.split_whitespace();
            let sub = parts.next().unwrap_or("");
            match sub {
                "" => {
                    out.push(format!(
                        "current provider: {} (model {}) · usage: /provider <name>",
                        runtime.config.provider, runtime.config.model
                    ));
                    out.push(format!(
                        "providers: {} · manage: /provider add|rm|list",
                        catalog::provider_names().join(", ")
                    ));
                }
                "add" => {
                    let name = parts.next().unwrap_or("");
                    let base_url = parts.next().unwrap_or("");
                    let default_model = parts.next().unwrap_or("");
                    if name.is_empty() || base_url.is_empty() {
                        out.push(
                            "usage: /provider add <name> <base_url> [default_model]".to_string(),
                        );
                    } else {
                        let mut store = UserProviders::load();
                        let replaced = store.upsert(UserProvider {
                            name: name.to_string(),
                            base_url: base_url.to_string(),
                            default_model: default_model.to_string(),
                            models: if default_model.is_empty() {
                                Vec::new()
                            } else {
                                vec![default_model.to_string()]
                            },
                            removed: false,
                            prompt_cache: None,
                        });
                        match store.save() {
                            Ok(()) => out.push(format!(
                                "provider `{}` {} (providers.json)",
                                name,
                                if replaced { "updated" } else { "added" }
                            )),
                            Err(e) => out.push(format!("[error] {}", e)),
                        }
                    }
                }
                "rm" => {
                    let name = parts.next().unwrap_or("");
                    if name.is_empty() {
                        out.push("usage: /provider rm <name>".to_string());
                    } else {
                        let mut store = UserProviders::load();
                        if store.remove(name) {
                            match store.save() {
                                Ok(()) => out.push(format!("provider `{}` removed", name)),
                                Err(e) => out.push(format!("[error] {}", e)),
                            }
                        } else {
                            out.push(format!("no user provider named `{}`", name));
                        }
                    }
                }
                "list" => {
                    let all = catalog::all_providers();
                    out.push(format!("providers ({}):", all.len()));
                    for p in all {
                        let tag = if p.user_defined { " (custom)" } else { "" };
                        out.push(format!(
                            "  {}{} · {} · default `{}`",
                            p.name, tag, p.base_url, p.default_model
                        ));
                    }
                }
                _ => {
                    if let Some(default_model) = catalog::default_model(sub) {
                        runtime.switch_model(sub, &default_model)?;
                        out.push(format!("provider → {} · model → {}", sub, default_model));
                        out.push("selection saved to config.json (global)".to_string());
                        if !runtime.has_token_for(sub) {
                            out.push(format!(
                                "no token for provider `{}` — use /auth {} to add one",
                                sub, sub
                            ));
                        }
                    } else {
                        out.push(format!(
                            "unknown provider: {} (options: {})",
                            sub,
                            catalog::provider_names().join(", ")
                        ));
                    }
                }
            }
        }
        "/auth" => {
            use crate::harness::auth::AuthStore;
            // No argument → update the token for the current provider/model.
            let provider = if arg.is_empty() {
                runtime.config.provider.clone()
            } else {
                arg.to_string()
            };
            if provider.is_empty() {
                let store = AuthStore::load();
                let names = store.entries.keys().cloned().collect::<Vec<_>>();
                out.push("usage: /auth <provider> — stored providers:".to_string());
                if names.is_empty() {
                    out.push("  (none)".to_string());
                } else {
                    out.push(format!("  {}", names.join(", ")));
                }
            } else {
                // Prompt goes straight to stdout so it shows before blocking on input.
                println!("paste the API key for `{}`: ", provider);
                let key = tokio::task::spawn_blocking(|| -> String {
                    let mut s = String::new();
                    if std::io::stdin().read_line(&mut s).is_ok() {
                        s.trim().to_string()
                    } else {
                        String::new()
                    }
                })
                .await
                .unwrap_or_default();
                if key.trim().is_empty() {
                    out.push("empty token — auth cancelled".to_string());
                } else {
                    let mut store = AuthStore::load();
                    store.set_key(&provider, key.trim());
                    match store.save() {
                        Ok(()) => out.push(format!(
                            "token saved for `{}` (auth.json, chmod 600)",
                            provider
                        )),
                        Err(e) => out.push(format!("[error] failed to save token: {}", e)),
                    }
                }
            }
        }
        _ => unreachable!("handle_provider_cmd called with unknown cmd: {}", cmd),
    }
    Ok(())
}

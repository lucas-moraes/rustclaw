# TODO — Rodada C: Prompt Caching (C1–C6)

> **Data:** 2026-09-09 · **Base:** `main` pós-commit `47ec851` (split app.rs)
> **Objetivo:** cachear o prefixo do prompt (system + tools + histórico) para
> cortar 50–90% do custo de input em sessões longas, e exibir/contabilizar
> tokens de cache no `/usage` e na sidebar.
> **Complementa:** `ASSESSMENT.md` §1.5 (Prioridade 1).

---

## 📋 Diagnóstico (estado atual)

| Problema | Onde | Efeito |
|---|---|---|
| `Usage` sem campos de cache | `provider/mod.rs:20` | tokens de cache invisíveis |
| OpenAI streaming sem `stream_options` | `provider/openai.rs` (body do stream) | **usage nunca chega** no streaming OpenAI (a API só envia `usage` com `include_usage: true`) |
| `cached_tokens` não parseado | `openai.rs:167,294` | economia OpenAI invisível |
| `cache_read/creation_input_tokens` não parseados | `anthropic.rs:155,171` | economia Anthropic invisível |
| Sem `cache_control` no body | `anthropic.rs` | Anthropic não cacheia nada |
| **System prompt muda a cada turno** | `runtime.rs:575 project_context_for` — memory facts re-ranqueados por query + summary regenerado quando `needs_regen` | **invalida todo o cache de prefixo** (Anthropic exige match byte-a-byte; OpenAI ≥1024 tokens idênticos) |
| MiniMax via adapter Anthropic | `opencode_go.rs:28` | `cache_control` pode ser rejeitado → precisa gating |
| `estimate_cost` não cache-aware | `catalog.rs:259` | custo exibido errado com cache ativo |

**Semântica por provider (não uniformizar!):**
- **Anthropic:** `input_tokens` **EXCLUI** cache; campos separados
  `cache_creation_input_tokens` (write) e `cache_read_input_tokens` (read).
  Preço: write = 1.25× input, read = 0.1× input.
- **OpenAI:** `prompt_tokens` **INCLUI** o que foi cacheado;
  `prompt_tokens_details.cached_tokens` é o subconjunto lido do cache.
  Preço: cached = 0.5× input (default; alguns modelos 0.25×/0.1×).
  Caching é **automático** (não há flag de request) — só exige prefixo estável.

---

## C1 — `Usage` com cache + parsing por provider

**Arquivos:** `provider/mod.rs`, `provider/anthropic.rs`, `provider/openai.rs`,
`session/processor.rs`

- [ ] `Usage` ganha `cache_read_tokens: u64` e `cache_write_tokens: u64`
      (default 0 em todos os construtores); `add_assign` soma os 4 campos;
      novo helper `cache_total() -> u64`.
- [ ] `anthropic.rs`: parsear `cache_read_input_tokens` → read e
      `cache_creation_input_tokens` → write em **ambos** os caminhos
      (não-streaming `parse_response` ~L155 e streaming state ~L171/191/309).
      **Não** somar ao `input_tokens` (semântica Anthropic).
- [ ] `openai.rs`: parsear `usage.prompt_tokens_details.cached_tokens` →
      `cache_read_tokens` (write = 0) nos dois caminhos. `prompt_tokens`
      continua como está (já inclui cached — não somar de novo).
- [ ] `openai.rs` streaming: adicionar `"stream_options": {"include_usage": true}`
      ao body quando `stream: true`. Defensivo: se a resposta for 400 e o body
      mencionar `stream_options`, retry 1× sem o campo (alguns proxies
      OpenAI-compat não o suportam).
- [ ] `processor.rs`: acumular `cache_read_tokens`/`cache_write_tokens` nos
      mesmos pontos que somam input/output (~L280, L328, L335) e incluir no
      log `turn end` (~L501).
- [ ] Testes: fixture Anthropic com os 3 campos de usage (não-stream + SSE);
      fixture OpenAI com `prompt_tokens_details`; teste de `add_assign` com cache.

**Aceitação:** `cargo test provider` passa; streaming OpenAI agora reporta usage.

---

## C2 — Estabilidade do prefixo (pré-requisito do cache)

**Arquivos:** `runtime.rs`, `project/memory.rs`, `agent/mod.rs`,
`ui/tui/app/events.rs`, `ui/cli.rs`, `session/compaction.rs`

O cache só funciona se o prefixo for **byte-estável entre turnos**. Hoje duas
coisas quebram isso no system prompt: o summary estrutural (regenera quando o
profiler detecta mudança — ou seja, a cada turno com edits) e o bloco de memory
(re-ranqueado por query a cada turno).

- [ ] **Congelar o summary por sessão:** `project_context_for` separa em
      (a) summary estrutural — computado **1× na primeira chamada da sessão**
      (cache em `SessionRuntime`, `HashMap<session_id, String>`; ao resumir uma
      sessão, recomputa e congela dali em diante) e (b) memory block (por turno).
      Regenerar o summary apenas após **compaction** (o contexto foi reescrito
      de qualquer forma). Trade-off aceito: summary pode ficar stale durante
      edits — o agente tem glob/grep/read para informação fresca.
- [ ] **Mover o bloco de memory para o prefixo da user message** (persistida),
      com marcadores:
      ```
      <project-memory>
      …facts ranqueados por query…
      </project-memory>
      ```
      como `Part::Text` separado **antes** do texto do usuário. O ranqueamento
      continua usando `user_text`. O system prompt fica 100% estável
      (agent + cwd + skills + tools + summary congelado).
- [ ] Helper `is_memory_block(text: &str) -> bool` em `project/memory.rs`.
- [ ] **Strip no UI:** transcript rebuild (`app/events.rs`) e CLI (`ui/cli.rs`)
      pulam parts que começam com `<project-memory>`. (Compaction deixa como
      está — o bloco é resumido junto com a mensagem.)
- [ ] Nota: toggles de skills (`/skills`) e mudanças no tool set MCP
      invalidam o cache — legítimo (ação do usuário), documentar apenas.
- [ ] Testes: system prompt byte-idêntico em 2 turnos consecutivos; user message
      contém o bloco marcado; strip no transcript.

**Aceitação:** dois turnos seguidos produzem `system_prompt` idêntico
(`assert_eq!` no teste).

---

## C3 — Anthropic `cache_control` (3 breakpoints, gated)

**Arquivos:** `provider/anthropic.rs`, `provider/opencode_go.rs`,
`provider/catalog.rs` (construção do provider)

- [ ] `AnthropicProvider` ganha `prompt_cache: bool`.
      - `true` no provider Anthropic builtin (catalog).
      - `false` quando construído via `opencode_go.rs` (MiniMax `/messages`
        pode rejeitar `cache_control`); overridável via `providers.json`
        (`"prompt_cache": true`).
- [ ] `build_request_body` com flag ligada:
      1. `system` vira array de blocks: `[{"type":"text","text":…,
         "cache_control":{"type":"ephemeral"}}]` (breakpoint 1);
      2. último item de `tools` recebe `cache_control` (breakpoint 2);
      3. último content block da última message recebe `cache_control`
         (breakpoint 3 — cobre todo o histórico anterior).
      Limite da API: 4 breakpoints; usamos 3.
- [ ] Com flag desligada: body exatamente como hoje (system como string) —
      zero risco para MiniMax/opencode-go.
- [ ] Testes: snapshot do body com flag on (system array + 3 marcações) e
      off (idêntico ao atual); contagem de breakpoints ≤ 4.

**Aceitação:** `cargo test anthropic` passa; MiniMax inalterado.

---

## C4 — Custo cache-aware + exibição

**Arquivos:** `provider/catalog.rs`, `ui/tui/app/usage.rs`,
`ui/tui/draw/sidebar.rs`, `ui/tui/draw/status.rs`

- [ ] `catalog.rs`: `estimate_cost` vira cache-aware — nova assinatura
      `estimate_cost_cached(provider, model, &Usage) -> f64`:
      - Anthropic: `input + 1.25×write + 0.1×read` como input billable;
      - OpenAI: `(input − read) + 0.5×read` como input billable;
      - outros: comportamento atual (cache = 0).
      Manter `estimate_cost` antigo como wrapper (callers: `usage.rs` ×2).
- [ ] `usage.rs`: `record_usage`/`usage_report` usam a versão cached;
      `/usage` ganha linhas "cache read: X tok (Y% do input)" e
      "cache write: W tok".
- [ ] Sidebar/status: indicador compacto de cache (ex.: `↻12.3k` ao lado dos
      tokens de input da sessão) quando `cache_read_tokens > 0`.
- [ ] Testes: `estimate_cost_cached` para Anthropic (write 1.25×, read 0.1×)
      e OpenAI (read 0.5×, sem double-count).

**Aceitação:** `/usage` reflete economia real; custo da sidebar bate com a
conta manual.

---

## C5 — Config, providers.json e docs

**Arquivos:** `config.rs`, `provider/user_store.rs`, `provider/catalog.rs`,
`docs/FEATURES.md`, `README.md`, `AGENTS.md`

- [ ] `config.json`: kill-switch global `"prompt_caching": true` (default ON).
      Quando OFF: `prompt_cache=false` em todos os providers (útil para proxies
      que engasgam com `cache_control` mesmo em endpoint Anthropic-compat).
- [ ] `providers.json`: campo opcional `"prompt_cache": bool` por provider
      (merge no catalog; builtin Anthropic default true, resto false).
- [ ] Precedência: `config.json.prompt_caching` (global) →
      `providers.json.prompt_cache` (por provider).
- [ ] Docs: `docs/FEATURES.md` §Prompt caching (semântica por provider,
      breakpoints, estabilidade de prefixo, como medir no `/usage`); bullet no
      README; nota no AGENTS.md §Provider.

---

## C6 — Verificação final

- [ ] `cargo test` (todos os novos testes C1–C4)
- [ ] `cargo clippy -- -D warnings`
- [ ] `cargo fmt --check`
- [ ] Smoke manual (opcional, requer key real): sessão de 2+ turnos com
      Anthropic → turn 2+ deve reportar `cache_read_input_tokens > 0`;
      com OpenAI → `cached_tokens > 0` (prefixo ≥1024 tokens — nosso system
      prompt com AGENTS.md + skills + tools excede com folga).
- [ ] Atualizar este arquivo marcando ✅ por item.

---

## 🔗 Ordem & dependências

```
C1 (Usage/parsing)  ──┐
C2 (prefixo estável) ──┼──► C3 (cache_control) ──► C4 (custo/exibição) ──► C5 (config/docs) ──► C6
```

C1 e C2 são independentes entre si e podem ir em paralelo; **C2 é pré-requisito
de C3** (sem prefixo estável, `cache_control` não hita nunca). C4 depende de C1
(campos) e C3 (para validar economia). Estimativa total: **3–4 dias**.

## ⚠️ Riscos / trade-offs

| Risco | Mitigação |
|---|---|
| MiniMax/proxy rejeita `cache_control` | flag `prompt_cache` default OFF fora do Anthropic builtin; kill-switch global |
| Proxy OpenAI-compat rejeita `stream_options` | retry 1× sem o campo em 400 |
| Bloco de memory na user message cresce o histórico (~500 tok/turn) | tokens de cache custam 0.1×/0.5×; compaction já existe; bloco é stripped no UI |
| Summary congelado fica stale durante edits | agente tem glob/grep/read; regenera pós-compaction |
| Compaction reescreve histórico → cache miss | legítimo e único (1 miss por compaction) |

## 🚫 Fora de escopo

- Gemini context caching (API diferente — rodada futura)
- Painel de diagnóstico de cache no TUI (o `/usage` já cobre)
- Ajuste fino de breakpoints >3 (Anthropic cobra write; 3 é o sweet spot)

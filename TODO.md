# RustClaw — Resolver streaming que não retorna durante implementação de features

> **Problema:** durante a implementação de uma lista de features, o sistema fica preso em
> streaming e não retorna. A causa raiz é a **falta de timeout no client HTTP do provider**
> (`reqwest::Client::new()` sem timeout), que faz o `bytes.next().await` ficar pendurado
> indefinidamente quando o servidor mantém a conexão aberta sem enviar dados nem `[DONE]`.
>
> **Escopo:** 4 fases independentes que eliminam os 3 caminhos de hang identificados.

---

## Visão geral das features

| ID | Feature | Fase | Status |
|----|---------|------|--------|
| S0 | Timeout no client HTTP do provider (causa raiz) | 1 | ✅ |
| S1 | Timeout na compactação (`provider.complete`) | 2 | ✅ |
| S2 | `SseParser` robusto para `\r\n\r\n` | 3 | ✅ |
| S3 | Timeout de segurança no processor (stream) | 4 | ✅ |
| S4 | Verificação final + commit | 5 | ⬜ |

**Legenda:** ⬜ pendente · 🟡 em progresso · ✅ feito · ❌ cancelado

---

## S0 — Timeout no client HTTP do provider (causa raiz)

**Objetivo:** impedir que o `bytes.next().await` fique pendurado indefinidamente quando o
servidor mantém a conexão aberta sem enviar dados. Resolve diretamente o sintoma relatado.

### Feature: Helper `build_http_client()`

- [x] Adicionar `use std::time::Duration;` em `src/harness/provider/mod.rs`
- [x] Criar função `pub fn build_http_client() -> reqwest::Client`:
  ```rust
  pub fn build_http_client() -> reqwest::Client {
      reqwest::Client::builder()
          .connect_timeout(Duration::from_secs(30))
          .read_timeout(Duration::from_secs(120)) // tempo entre chunks
          .build()
          .expect("failed to build http client")
  }
  ```
- [x] Usar **`read_timeout`** (tempo entre chunks) em vez de `timeout` (tempo total) —
      streaming de raciocínio pode durar minutos e um timeout total mataria respostas legítimas
- [x] `connect_timeout` de 30s para falhar rápido em conexões mortas

### Feature: Aplicar nos pontos de produção

- [x] `src/harness/runtime.rs:137` (`from_legacy`): substituir `reqwest::Client::new()` por
      `crate::harness::provider::build_http_client()`
- [x] `src/harness/runtime.rs:194` (`switch_model_with_auth`): idem

### Feature: Consistência nos testes (opcional)

- [x] Atualizar `reqwest::Client::new()` em testes para `build_http_client()`:
  - [x] `runtime.rs:629` (test_runtime)
  - [x] `runtime.rs:750` (test_onboarding)
  - [x] `app.rs:560` (test TUI)
  - [x] `memory.rs:74` (test memory)
  - [x] `opencode_go.rs:92` (test_build_provider_routing)

### Definition of done S0

- [x] `cargo check` verde
- [x] `cargo test` verde (sem regressão)
- [x] Nenhum `reqwest::Client::new()` sem timeout em código de produção
- [x] `cargo clippy --bin rustclaw` sem novos warnings

---

## S1 — Timeout na compactação

**Objetivo:** impedir que a compactação (frequente durante implementação de features, quando o
contexto estoura) fique presa numa chamada `provider.complete()` sem timeout.

### Feature: Timeout no `summarize()`

- [x] Em `src/harness/session/compaction.rs`, adicionar `use std::time::Duration;`
- [x] Envolver `provider.complete(&summary_req).await` (linha 90) em
      `tokio::time::timeout(Duration::from_secs(120), ...)`
- [x] Tratar o resultado:
  - [x] `Ok(Ok(resp))` → fluxo normal (extrair texto)
  - [x] `Ok(Err(e))` → fallback `"(summary unavailable)"` (caminho já existe)
  - [x] `Err(_)` (timeout) → `tracing::warn!` + fallback `"(summary unavailable)"`
- [x] Garantir que o fallback de timeout não propaga erro para o processor (compactação nunca
      deve travar o turno)

### Feature: Teste de timeout

- [x] Adicionar teste em `compaction.rs` com um `MockProvider` que **nunca responde** (ex:
      `tokio::time::sleep(Duration::from_secs(3600))` no `complete`)
- [x] Verificar que `should_compact_and_execute` retorna `Some(...)` com
      `"(summary unavailable)"` após o timeout (não trava)

### Definition of done S1

- [x] `cargo test` verde (incl. novo teste de timeout)
- [x] Compactação nunca trava o turno, mesmo com provedor lento
- [x] `cargo clippy --bin rustclaw` sem novos warnings

---

## S2 — `SseParser` robusto para `\r\n\r\n`

**Objetivo:** evitar que um provedor que use o padrão SSE `\r\n\r\n` (em vez de `\n\n`) faça o
parser nunca produzir eventos e o stream ficar preso esperando um delimitador que nunca chega.

### Feature: Normalizar `\r\n` → `\n` no buffer

- [x] Em `src/harness/provider/mod.rs`, no `SseParser::push`, normalizar o buffer antes de
      procurar o delimitador:
  - [x] Substituir `\r\n` por `\n` no chunk recebido antes de `extend_from_slice`
  - [x] **Seguro** porque os dados JSON escapam `\r\n` como `\\r\\n` literal — não há colisão
- [x] Alternativa (se preferir não mutar o chunk): procurar por `\n\n` **ou** `\r\n\r\n` no
      `find_subslice` — avaliar qual é mais simples/robusto

### Feature: Testes para `\r\n\r\n`

- [x] Adicionar teste: `parser.push(b"data: {\"a\":1}\r\n\r\n")` → `["{\"a\":1}"]`
- [x] Adicionar teste: `parser.push(b"data: [DONE]\r\n\r\n")` → `["[DONE]"]`
- [x] Adicionar teste: chunk dividido com `\r\n\r\n` (ex: `data: {\"a\":` + `1}\r\n\r\n`)
- [x] Garantir que os testes existentes de `\n\n` continuam passando

### Definition of done S2

- [x] `cargo test` verde (incl. novos testes `\r\n\r\n`)
- [x] `SseParser` produz eventos corretamente com `\n\n` e `\r\n\r\n`
- [x] `cargo clippy --bin rustclaw` sem novos warnings

---

## S3 — Timeout de segurança no processor (stream)

**Objetivo:** camada extra de segurança caso o `read_timeout` do client não cubra algum caso
(provedor custom, proxy, etc.). Garante que o `while let Some(ev) = stream.next().await` nunca
fique preso para sempre.

### Feature: Timeout no `stream.next().await`

- [x] Em `src/harness/session/processor.rs`, adicionar `use std::time::Duration;`
- [x] Envolver o `stream.next().await` (linha 111) em
      `tokio::time::timeout(Duration::from_secs(300), stream.next())`
- [x] Tratar o resultado:
  - [x] `Ok(Some(ev))` → processar evento normalmente (fluxo atual)
  - [x] `Ok(None)` → stream terminou, sair do `while`
  - [x] `Err(_)` (timeout) → emitir `HarnessEvent::Error` com mensagem clara e **quebrar o loop**
- [x] Usar **timeout generoso (300s)** para não interromper streaming de raciocínio longo
- [x] Ao quebrar por timeout, garantir que `final_text` receba mensagem de erro (não vazio)

### Feature: Teste (opcional, se viável)

- [ ] Se houver infraestrutura de mock de stream, adicionar teste com stream que nunca emite
      `End` nem `None` e verificar que o timeout dispara
- [x] Se não for viável com a infra atual, documentar o comportamento no código (comentário)

### Definition of done S3

- [x] `cargo test` verde
- [x] `stream.next().await` nunca fica preso para sempre (timeout de 300s)
- [x] Timeout emite erro visível ao usuário e libera a UI
- [x] `cargo clippy --bin rustclaw` sem novos warnings

---

## S4 — Verificação final + commit

**Objetivo:** garantir que tudo compila, testa e está documentado antes do commit.

### Feature: Build e lint

- [ ] `cargo fmt`
- [ ] `cargo check`
- [ ] `cargo test` (todos os testes)
- [ ] `cargo clippy --bin rustclaw` (sem novos warnings)

### Feature: Smoke test manual (se possível)

- [ ] Rodar `cargo run` e iniciar uma implementação de features
- [ ] Verificar que o streaming retorna normalmente (sem hang)
- [ ] Verificar que a compactação não trava
- [ ] Verificar que Ctrl+C ainda cancela o run

### Feature: Commit

- [ ] `git add -A`
- [ ] Commit com mensagem descritiva, ex:
      `fix(provider): add timeouts to prevent streaming hangs during feature implementation`
- [ ] Corpo do commit listando as 4 fases (S0–S3)

### Definition of done S4

- [ ] Todos os testes verdes
- [ ] Clippy sem novos warnings
- [ ] Commit criado

---

## Ordem de execução

```text
S0 timeout client HTTP (causa raiz)
 → S1 timeout compactação
 → S2 SseParser \r\n\r\n
 → S3 timeout processor (segurança)
 → S4 verificação + commit
```

Cada fase: `cargo test` + `cargo check` (+ `cargo clippy` no final).

---

## Riscos e mitigações

| Risco | Mitigação | Status |
|-------|-----------|--------|
| `read_timeout` de 120s mata streaming de raciocínio longo | Usar `read_timeout` (entre chunks), não `timeout` total; 120s é generoso | ⬜ |
| Timeout de 300s no processor interrompe resposta legítima | Generoso; só dispara se o client `read_timeout` falhar | ⬜ |
| Normalizar `\r\n`→`\n` corrompe JSON com `\r\n` literal | JSON escapa como `\\r\\n`; sem colisão real | ⬜ |
| Compactação com timeout retorna summary ruim | Fallback `"(summary unavailable)"` já existe; melhor que travar | ⬜ |
| Teste de timeout de compactação demora 120s | Usar timeout curto no teste (ex: 1s) via config ou mock | ⬜ |

---

## Arquivos principais a tocar

| Path | Mudança |
|------|---------|
| `src/harness/provider/mod.rs` | `build_http_client()` + `SseParser` `\r\n\r\n` |
| `src/harness/runtime.rs` | usar `build_http_client()` (linhas 137, 194) |
| `src/harness/session/compaction.rs` | timeout no `summarize()` + teste |
| `src/harness/session/processor.rs` | timeout no `stream.next().await` |
| `src/harness/provider/mod.rs` (tests) | testes `\r\n\r\n` |

---

## Notas de progresso

| Data | Nota |
|------|------|
| 2026-09-04 | TODO.md recriado: plano de resolução do hang de streaming convertido em features S0–S4 com checklists detalhados. Conteúdo anterior (TUI T0–T7, já concluído) substituído. |

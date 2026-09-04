# RustClaw — Gerenciamento de memória persistente do sistema

> **Problema:** a memória persistente hoje é **append-only e sem curadoria**. A tool `remember`
> só acumula linhas no `summary` do projeto (`ProjectMemoryStore`), que cresce sem limite e
> degrada a qualidade do contexto injetado no system prompt. Não há metadados por fato, ranking
> por relevância, deduplicação, retenção nem promoção de fatos importantes para memória
> permanente (skills).
>
> **Escopo:** 3 fases que evoluem o sistema existente (`ProjectMemoryStore` + `skill/`) em vez de
> criar um paralelo: metadados → ranking/injeção seletiva → GC/compactação.

---

## Visão geral das features

| ID | Feature | Fase | Status |
|----|---------|------|--------|
| M0 | Metadados por fato (schema + tool `remember`) | 1 | ✅ |
| M1 | Ranking por uso + injeção seletiva com orçamento | 2 | ✅ |
| M2 | Promoção de fato → SKILL.md permanente | 2 | ✅ |
| M3 | GC: dedup + arquivamento de itens não usados | 3 | ✅ |
| M4 | Compactação do `summary` acima do limite | 3 | ✅ |
| M5 | Verificação final + commit | 4 | ✅ |

**Legenda:** ⬜ pendente · 🟡 em progresso · ✅ feito · ❌ cancelado

---

## M0 — Metadados por fato (schema + tool `remember`)

**Objetivo:** dar a cada fato de memória metadados estruturados (`kind`, `confidence`,
`hit_count`, `last_used`, `archived`) para habilitar ranking, curadoria e retenção nas fases
seguintes. É a base de tudo.

### Feature: Migração do schema da tabela `memory`

- [x] Em `src/harness/project/memory.rs`, adicionar colunas à tabela `memory` (via migração
      idempotente, estilo `migrate_legacy`):
  - [x] `kind TEXT NOT NULL DEFAULT 'fact'` — convenção / comando / padrão / decisão / armadilha
  - [x] `confidence TEXT NOT NULL DEFAULT 'inferred'` — `inferred` | `confirmed`
  - [x] `hit_count INTEGER NOT NULL DEFAULT 0`
  - [x] `last_used TEXT` (timestamp ISO, nullable)
  - [x] `archived INTEGER NOT NULL DEFAULT 0` (0 = ativo, 1 = arquivado)
- [x] Manter compatibilidade com linhas existentes (defaults preenchem os novos campos)
- [x] Atualizar `ProjectMemoryRow` para expor os novos campos
- [x] Adicionar teste de migração: tabela antiga sem colunas novas → migra com defaults

### Feature: Tool `remember` com metadados

- [x] Em `src/harness/tool/remember.rs`, adicionar parâmetros opcionais ao schema JSON:
  - [x] `kind` (string, default `"fact"`)
  - [x] `confidence` (string, default `"inferred"`)
- [x] Validar valores: `kind` em conjunto conhecido, `confidence` em `inferred|confirmed`
- [x] Persistir os metadados na linha (não só no texto do `summary`)
- [x] Manter o comportamento append-only do texto (não quebrar testes existentes)
- [x] Adicionar teste: `remember` com `kind`/`confidence` persiste os metadados

### Feature: `/memory list` com metadados

- [x] Em `src/harness/ui/commands/memory.rs`, exibir `kind`, `confidence`, `hit_count` e
      `last_used` na listagem
- [x] Manter a numeração 1-indexada atual (usada por `rm`)
- [x] Adicionar teste: listagem formata os metadados

### Definition of done M0

- [x] `cargo test` verde (incl. novos testes de migração, remember e list)
- [x] `cargo check` verde
- [x] `cargo clippy --bin rustclaw` sem novos warnings
- [x] Tabela `memory` com colunas novas; linhas antigas migradas com defaults

---

## M1 — Ranking por uso + injeção seletiva com orçamento

**Objetivo:** em vez de despejar toda a memória no system prompt, selecionar os top-N fatos por
relevância (recência + `hit_count` + match lexical com a pergunta atual), respeitando um
orçamento de bytes. Fatos usados sobem; nunca usados caem.

### Feature: Registro de uso (`hit_count` / `last_used`)

- [x] Em `ProjectMemoryStore`, adicionar método `bump_usage(cwd, id)` que incrementa
      `hit_count` e atualiza `last_used`
- [x] Chamar `bump_usage` quando um fato é injetado no system prompt
- [x] Adicionar teste: `bump_usage` incrementa e atualiza timestamp

### Feature: Score de relevância

- [x] Criar função de score por fato: `score = w1*recency + w2*hit_count + w3*lexical_match`
- [x] `recency` decai com o tempo desde `last_used` (fatos recentes pesam mais)
- [x] `lexical_match` = sobreposição de tokens entre o fato e a pergunta/turno atual
- [x] Adicionar teste unitário do score (ordenação esperada)

### Feature: Injeção seletiva com orçamento

- [x] Em `src/harness/agent/mod.rs` (ou onde o `project_context` é montado), substituir o
      despejo total por seleção:
  - [x] Ordenar fatos ativos por score (desc)
  - [x] Consumir fatos até atingir o orçamento de bytes (ex. `MAX_MEMORY_CHARS = 2048`)
  - [x] Ficar com o texto truncado por UTF-8 (reusar padrão de `truncate_utf8` de `inject.rs`)
- [x] Garantir que fatos arquivados (`archived=1`) nunca entram na injeção
- [x] Adicionar teste: com orçamento pequeno, só os top-N entram

### Definition of done M1

- [x] `cargo test` verde (incl. testes de score, bump e injeção seletiva)
- [x] `cargo check` verde
- [x] `cargo clippy --bin rustclaw` sem novos warnings
- [x] System prompt recebe no máximo `MAX_MEMORY_CHARS` de memória, ordenada por relevância

---

## M2 — Promoção de fato → SKILL.md permanente

**Objetivo:** permitir que um fato importante (confidence alto + confirmado pelo usuário) saia
do lixo da sessão e vire um `SKILL.md` permanente no `.agents/skills/`, entrando no catálogo de
skills reutilizáveis.

### Feature: Comando `/memory promote <id>`

- [x] Em `src/harness/ui/commands/memory.rs`, adicionar subcomando `promote <id>`:
  - [x] Ler o fato pelo índice (mesma numeração do `list`)
  - [x] Gerar um `SKILL.md` em `<cwd>/.agents/skills/<slug>/SKILL.md` com frontmatter
        (`name`, `description`) + corpo = texto do fato
  - [x] `slug` derivado do texto (reusar `sanitize_id` de `loader.rs`)
  - [x] Marcar o fato como `archived=1` após a promoção (não duplicar no contexto)
- [x] Adicionar teste: `promote` cria o SKILL.md e arquiva o fato

### Feature: Validação e feedback

- [x] Erro claro se o índice for inválido ou o fato já estiver arquivado
- [x] Mensagem de sucesso apontando o caminho do SKILL.md criado
- [x] Adicionar teste: `promote` com índice inválido retorna erro

### Definition of done M2

- [x] `cargo test` verde (incl. testes de promote)
- [x] `cargo check` verde
- [x] `cargo clippy --bin rustclaw` sem novos warnings
- [x] Fato promovido vira skill no catálogo e sai da memória ativa

---

## M3 — GC: dedup + arquivamento de itens não usados

**Objetivo:** limpar a memória automaticamente: fundir fatos duplicados/semelhantes e arquivar
fatos nunca usados há muito tempo, mantendo o contexto enxuto.

### Feature: Deduplicação

- [x] Em `ProjectMemoryStore`, adicionar método `dedup(cwd)`:
  - [x] Agrupar fatos por similaridade (normalização de texto: lowercase, trim, remover
        timestamp)
  - [x] Fundir duplicados: manter o mais recente, somar `hit_count`, arquivar os demais
  - [x] Adicionar teste: dois fatos iguais → um ativo com `hit_count` somado

### Feature: Arquivamento por inatividade

- [x] Adicionar método `archive_stale(cwd, max_age_days)`:
  - [x] Arquivar fatos com `last_used` mais antigo que `max_age_days` (default 60) e
        `hit_count == 0`
  - [x] Nunca arquivar fatos com `confidence == "confirmed"`
  - [x] Adicionar teste: fato antigo sem uso é arquivado; confirmado não é

### Feature: Comando `/memory gc`

- [x] Em `src/harness/ui/commands/memory.rs`, adicionar subcomando `gc`:
  - [x] Executar `dedup` + `archive_stale` e reportar quantos itens foram fundidos/arquivados
  - [x] Adicionar teste: `gc` reporta contagens

### Definition of done M3

- [x] `cargo test` verde (incl. testes de dedup, archive_stale e gc)
- [x] `cargo check` verde
- [x] `cargo clippy --bin rustclaw` sem novos warnings
- [x] `/memory gc` funde duplicados e arquiva itens não usados, preservando confirmados

---

## M4 — Compactação do `summary` acima do limite

**Objetivo:** quando o `summary` do projeto passar de um limite, rolar os fatos antigos/baixa
prioridade para um blob comprimido, mantendo só o essencial no contexto.

### Feature: Limite e rolagem

- [x] Definir `MAX_SUMMARY_CHARS` (ex. 4096) para o `summary` ativo
- [x] Adicionar método `compact(cwd)`:
  - [x] Se `summary` ≤ limite, não faz nada
  - [x] Senão, mover os fatos de menor score (mais antigos / menos usados) para uma coluna
        `archive` (texto comprimido ou JSON), mantendo os top-N no `summary`
  - [x] Fatos `confirmed` nunca são rolados para o archive
- [x] Adicionar coluna `archive TEXT` na tabela `memory` (migração idempotente)
- [x] Adicionar teste: summary acima do limite → top-N mantidos, resto no archive

### Feature: Integração com GC

- [x] `/memory gc` também chama `compact(cwd)` ao final
- [x] Adicionar teste: `gc` compacta quando necessário

### Definition of done M4

- [x] `cargo test` verde (incl. testes de compact)
- [x] `cargo check` verde
- [x] `cargo clippy --bin rustclaw` sem novos warnings
- [x] `summary` nunca excede `MAX_SUMMARY_CHARS`; confirmados preservados

---

## M5 — Verificação final + commit

**Objetivo:** garantir que tudo compila, testa e está documentado antes do commit.

### Feature: Build e lint

- [x] `cargo fmt`
- [x] `cargo check`
- [x] `cargo test` (todos os testes)
- [x] `cargo clippy --bin rustclaw` (sem novos warnings)

### Feature: Smoke test manual (se possível)

- [ ] Rodar `cargo run` e usar `/memory list` para ver metadados
- [ ] Usar a tool `remember` com `kind`/`confidence` e confirmar persistência
- [ ] Rodar `/memory promote <id>` e ver o SKILL.md criado
- [ ] Rodar `/memory gc` e ver dedup/arquivamento/compactação
- [ ] Verificar que o system prompt não estoura o orçamento de memória

### Feature: Commit

- [x] `git add -A`
- [x] Commit com mensagem descritiva, ex:
      `feat(memory): metadata, ranking, promotion and GC for persistent project memory`
- [x] Corpo do commit listando as fases (M0–M4)

### Definition of done M5

- [x] Todos os testes verdes
- [x] Clippy sem novos warnings
- [x] Commit criado

---

## Ordem de execução

```text
M0 metadados (schema + remember + list)
 → M1 ranking + injeção seletiva
 → M2 promoção → SKILL.md
 → M3 GC (dedup + arquivamento)
 → M4 compactação do summary
 → M5 verificação + commit
```

Cada fase: `cargo test` + `cargo check` (+ `cargo clippy` no final).

---

## Riscos e mitigações

| Risco | Mitigação | Status |
|-------|-----------|--------|
| Migração do schema quebra linhas existentes | Defaults preenchem novos campos; migração idempotente estilo `migrate_legacy` | ⬜ |
| Injeção seletiva omite fato importante | Score inclui `confidence`/`confirmed` que nunca são arquivados/rolados | ⬜ |
| Dedup funde fatos distintos por engano | Normalização conservadora + manter o mais recente; revisão manual via `/memory list` | ⬜ |
| Compactação perde contexto útil | Fatos `confirmed` nunca vão para o archive; archive é recuperável | ⬜ |
| Orçamento de memória muito pequeno | `MAX_MEMORY_CHARS` configurável; default 2048 | ⬜ |
| Promoção gera SKILL.md duplicado | `sanitize_id` + arquivar o fato original após promover | ⬜ |

---

## Arquivos principais a tocar

| Path | Mudança |
|------|---------|
| `src/harness/project/memory.rs` | migração schema (kind/confidence/hit_count/last_used/archived/archive) + dedup/archive_stale/compact/bump_usage |
| `src/harness/tool/remember.rs` | parâmetros `kind`/`confidence` + persistência de metadados |
| `src/harness/ui/commands/memory.rs` | `/memory list` com metadados + `promote` + `gc` |
| `src/harness/agent/mod.rs` | injeção seletiva com orçamento + score de relevância |
| `src/harness/skill/loader.rs` | reuso de `sanitize_id` para slug de promoção |

---

## Notas de progresso

| Data | Nota |
|------|------|
| 2026-09-04 | TODO.md recriado: plano de gerenciamento de memória persistente convertido em features M0–M5 com checklists detalhados. Conteúdo anterior (S0–S4, já concluído) substituído. |
| 2026-09-04 | M0–M5 implementados: tabela de fatos com metadados (M0), score de relevância + injeção seletiva com orçamento (M1), `/memory promote` → SKILL.md (M2), `/memory gc` com dedup + archive_stale (M3), `compact()` de fatos ativos (M4), verificação+commit (M5). 187 testes verdes; clippy sem novos warnings. Commit `572ec0d`. |

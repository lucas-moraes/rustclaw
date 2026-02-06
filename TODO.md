# Fases para desenvolvimento do projeto rustclaw
---

<details>
<summary>Fase 0: Setup e Fundações (1-3 dias) (COMPLETO)</summary>
Criar projeto Cargo (cargo new rustclaw --bin)
Adicionar dependências principais:
rig (framework principal de agents)
tokio + tokio-util
async-openai / reqwest
serde, toml, tracing
fastembed-rust (embeddings leves)
oasysdb ou lancedb (vector store embedded)

Configurar .env (API keys: Kimi, Groq, OpenAI, Telegram Token)
Teste básico: Conexão com LLM externo (Kimi 2.5)

Testes:

Enviar prompt simples e receber resposta streaming.
</details>

<details>
<summary>Fase 1: Core Agent Loop + Tool Calling (3-5 dias) (COMPLETO)</summary>

Criar agente base com Rig
Implementar ReAct / Plan-and-Execute loop
Definir estrutura de ferramentas (Tool trait)

Testes:

Agente responde perguntas gerais
Agente reconhece e chama ferramentas (ex: calculadora)
</details>

<details>
<summary>Fase 2: Ferramentas Essenciais (Execução Real) (5-7 dias) (COMPLETO)</summary>

Ferramentas prioritárias:
Shell execution (std::process::Command)
File System (read, write, list, search)
HTTP Request (web search / APIs)
System info (RAM, CPU, disk usage)


Testes:

"Liste os arquivos da pasta atual"
"Crie um arquivo teste.txt com o texto 'hello'"
"Execute ls -la"

</details>

A partir daqui podemos seguir dois caminhos, assistente de desenvolvimento, ou assistente geral

## Fase 3: Memória Persistente de Longo Prazo (LTM) (6-8 dias)

Gerar embeddings com fastembed-rust
Usar OasysDB ou LanceDB como vector store (leves e embedded)
Implementar:
Short-term memory (contexto atual)
Long-term memory (RAG)
Memória de fatos/episódios (com timestamps e importância)


Testes:

"Lembra qual API eu prefiro usar?" (deve lembrar de conversa anterior)
Busca semântica funciona após reiniciar o programa

## Fase 4: Integração com Chat (Telegram) (4-6 dias)

Usar teloxide
Suporte a comandos (/start, /status, /clear_memory)
Modo conversacional persistente

Testes:

Receber mensagem no Telegram → agente responde
Agente mantém contexto entre várias mensagens

## Fase 5: Proatividade e Agendamento (4-6 dias)

Implementar scheduler (tokio-cron-scheduler)
Heartbeat diário (resumo do dia, tarefas pendentes)
Tarefas agendadas (ex: checar emails toda manhã)

Testes:

Agente envia mensagem automática todo dia às 8h
Heartbeat funciona após reboot

## Fase 6: Browser Automation (Opcional / Avançado) (5-7 dias)

Integrar headless_chrome ou fantoccini
Ferramentas: navegar, screenshot, extrair texto

Testes:

"Acesse google.com e busque por 'preço bitcoin'"
Tirar screenshot e salvar

## Fase 7: Otimização para Raspberry Pi 3 + Segurança (5-7 dias)

Compilação cruzada para armv7-unknown-linux-gnueabihf
Reduzir uso de RAM (limites de buffer, embeddings em batch)
Sandboxing básico para comandos shell
Logging rotacionado + graceful shutdown

Testes:

Uso de memória < 300-400 MB em idle
Estabilidade rodando 24h+ no Pi 3

## Fase 8: Testes Finais, CLI e Documentação (3-5 dias)

CLI commands (rustclaw start, rustclaw status, rustclaw add-task)
Testes de ponta a ponta (E2E)
README completo + exemplos

Milestones Gerais

MVP (Fases 0-4): Agente funcional via Telegram com memória → 3-4 semanas
Versão 1.0 (Fases 0-7): Proativo + browser → 6-8 semanas

Quer que eu detalhe mais alguma fase específica (ex: código exemplo da Fase 3 de memória)?

# Novas Funcionalidades: Data/Hora e Localização

## ✨ O que foi implementado

### 1. Ferramenta `datetime` - Data e Hora
**Arquivo:** `src/tools/datetime.rs`

**Função:** Obtém a data e hora atual do sistema

**Uso:**
```json
{}
// ou
{"format": "iso"}     // Formato ISO 8601
{"format": "date"}    // Apenas data
{"format": "time"}    // Apenas hora
```

**Retorno exemplo:**
```
Data: 09/02/2025
Hora: 14:30:25
Dia da semana: Sunday
Timezone: -03:00
```

### 2. Ferramenta `location` - Geolocalização
**Arquivo:** `src/tools/location.rs`

**Função:** Obtém a localização geográfica baseada no IP do dispositivo

**Serviços utilizados:**
- ipapi.co
- ipinfo.io (fallback)

**Retorno exemplo:**
```
Localização do dispositivo:
Cidade: São Paulo, SP, Brazil
País: Brazil
Coordenadas: 23° 33' S, 46° 38' W
Timezone: America/Sao_Paulo
IP: 189.xxx.xxx.xxx
```

**Quando usar:**
- Para saber o clima local
- Para informar fuso horário
- Para calcular distâncias
- Para contextualizar respostas baseadas em localização

---

## 🎯 Como usar

### Perguntas que o sistema agora pode responder:

**Data e Hora:**
- "Que horas são agora?"
- "Qual a data de hoje?"
- "Que dia da semana é hoje?"

**Localização:**
- "Onde estou?"
- "Qual é o meu fuso horário?"
- "Qual a previsão do tempo aqui?" (combinado com Tavily)
- "Que horas são em Tóquio?" (com cálculo de fuso)

**Contextualizadas:**
- "Devo levar guarda-chuva hoje?" → Busca clima local + data/hora
- "Qual o melhor horário para ligar para Nova York?" → Fuso horário local + destino

---

## 🔧 Funcionamento Técnico

### Data/Hora
- Usa `chrono::Local` para obter hora do sistema
- Detecta timezone automaticamente
- Formata em português

### Localização
- Consulta APIs públicas de geolocalização por IP
- Fallback automático entre múltiplos serviços
- Não requer GPS (funciona com qualquer conexão internet)
- Cache implícito (cada consulta é em tempo real)

---

## 📁 Arquivos Modificados

1. **`src/tools/datetime.rs`** (novo) - Ferramenta de data/hora
2. **`src/tools/location.rs`** (novo) - Ferramenta de geolocalização
3. **`src/tools/mod.rs`** - Registra novos módulos
4. **`src/cli.rs`** - Adiciona ferramentas no CLI
5. **`src/telegram/bot.rs`** - Adiciona ferramentas no Telegram
6. **`src/agent.rs`** - Atualiza prompt do sistema com diretrizes

---

## 🚀 Exemplos de Uso

### Exemplo 1: Que horas são?
```
Usuário: Que horas são agora?

AI:
Thought: O usuário quer saber a hora atual. Vou usar a ferramenta datetime.
Action: datetime
Action Input: {}

[Resultado da ferramenta]
Data: 09/02/2025
Hora: 14:30:25
Dia da semana: Sunday
Timezone: -03:00

AI:
Thought: Agora tenho a informação da hora. Vou responder.
Final Answer: São 14:30:25 do dia 09/02/2025 (domingo).
```

### Exemplo 2: Qual o clima aqui?
```
Usuário: Qual o clima aqui?

AI:
Thought: Para informar o clima local, preciso primeiro saber a localização.
Action: location
Action Input: {}

[Resultado: São Paulo, SP, Brazil]

AI:
Thought: Agora sei que estamos em São Paulo. Vou buscar o clima atual.
Action: tavily_search
Action Input: {"query": "previsão do tempo São Paulo hoje"}

[Resultado da busca]

AI:
Thought: Tenho as informações do clima. Vou responder.
Final Answer: Em São Paulo, SP, hoje está com temperatura de 28°C e...
```

### Exemplo 3: Que horas são em Tóquio?
```
Usuário: Que horas são agora em Tóquio?

AI:
Thought: Preciso saber meu fuso horário atual para calcular a diferença com Tóquio.
Action: location
Action Input: {}

[Resultado: Timezone: America/Sao_Paulo]

AI:
Thought: Estou em São Paulo (UTC-3), Tóquio é UTC+9, diferença de 12 horas.
Action: datetime
Action Input: {}

[Resultado: Hora: 14:30:25]

AI:
Thought: São 14:30 aqui, então em Tóquio são 14:30 + 12 = 02:30 do dia seguinte.
Final Answer: Aqui são 14:30. Em Tóquio são 02:30 do dia seguinte (12 horas à frente).
```

---

## ⚠️ Limitações

### Data/Hora
- Baseado no relógio do sistema Raspberry Pi
- Requer que o RPi tenha hora configurada corretamente (NTP)

### Localização
- Baseada em IP (precisão de cidade/região, não GPS exato)
- Requer conexão com internet
- Se usar VPN, mostrará localização do servidor VPN
- Se não tiver internet, retorna mensagem de erro amigável

---

## 🔒 Privacidade

- A ferramenta `location` consulta serviços públicos de geolocalização por IP
- O IP não é armazenado, apenas usado para consulta em tempo real
- Não rastreia o usuário continuamente
- Localização é obtida sob demanda quando solicitada

---

## 📝 Notas para Desenvolvedores

As ferramentas seguem o padrão `Tool` existente:
- Implementam trait `Tool` com `name()`, `description()` e `call()`
- São registradas no `ToolRegistry` em CLI e Telegram
- Usam `async_trait` para operações assíncronas
- Retornam `Result<String, String>`

A localização tenta múltiplos serviços automaticamente em caso de falha:
1. Tenta ipapi.co
2. Se falhar, tenta ipinfo.io
3. Se ambos falharem, retorna erro amigável

---

## ✅ Status

- [x] Ferramenta datetime criada
- [x] Ferramenta location criada
- [x] Integração com CLI
- [x] Integração com Telegram
- [x] Prompt do sistema atualizado
- [x] Build testado e funcionando
- [x] Documentação criada

**Pronto para usar!** 🎉

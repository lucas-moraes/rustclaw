# Sistema de Skills do RustClaw - Resumo Completo

## 🎯 Visão Geral

O RustClaw possui um sistema completo de gerenciamento de skills baseado em arquivos SKILL.md, com hot reload automático e detecção inteligente.

## 📁 Estrutura

```
skills/
├── general/
│   └── skill.md          # Skill padrão (fallback)
├── skill-manager/
│   └── skill.md          # Skill para gerenciar outras skills
└── [outras-skills]/
    └── skill.md          # Skills personalizadas
```

## 🛠️ Ferramentas Disponíveis

### 1. **skill_list**
Lista todas as skills disponíveis com descrições.
```json
{}
```

### 2. **skill_create**
Cria nova skill a partir de template.
```json
{ "name": "minha-skill", "custom_content": "opcional" }
```

### 3. **skill_edit**
Mostra conteúdo de uma skill para edição.
```json
{ "name": "minha-skill" }
```

### 4. **skill_validate**
Valida sintaxe de SKILL.md.
```json
{ "name": "minha-skill" }  // ou {} para todas
```

### 5. **skill_rename**
Renomeia uma skill existente.
```json
{ "old_name": "antigo", "new_name": "novo" }
```

### 6. **skill_delete**
Remove uma skill (com backup).
```json
{ "name": "minha-skill", "confirm": true }
```

### 7. **skill_import_from_url** ⭐ NOVO!
Importa e converte documentação de URL em skill.
```json
{ "url": "https://example.com/doc.md", "skill_name": "minha-skill" }
```

## 🚀 Funcionalidades

### Hot Reload Automático
- Skills são recarregadas automaticamente quando modificadas
- Sem necessidade de reiniciar o RustClaw
- Detecta novas skills adicionadas

### Detecção Inteligente
- Detecta skill baseado em keywords na mensagem
- Persiste skill ativa por chat no banco SQLite
- Evita mudanças frequentes (boost para skill atual)

### Validação Obrigatória
- Todas as operações validam o SKILL.md
- Rollback automático em caso de erro
- Proteção da skill 'general'

## 📝 Formato SKILL.md

```markdown
# Skill: nome-da-skill

## Descrição
Breve descrição do propósito

## Contexto
Contexto detalhado para o assistente usar

## Keywords
- keyword1
- keyword2
- keyword3

## Comportamento

### SEMPRE
- Comportamento obrigatório 1
- Comportamento obrigatório 2

### NUNCA
- Comportamento proibido 1
- Comportamento proibido 2

## Ferramentas Prioritárias
1. tool_name1
2. tool_name2

## Exemplos

### Input: "exemplo de pergunta"
**Bom:** resposta desejada
**Ruim:** resposta a ser evitada
```

## 🎨 Convenções

### Nomenclatura
- Use **kebab-case** (minúsculas com hífen)
- Exemplos: `code-reviewer`, `python-expert`, `meeting-assistant`
- Evite: espaços, CamelCase, snake_case

### Keywords
- Use 3-7 palavras-chave relevantes
- Inclua sinônimos
- Adicione termos técnicos do domínio

### Comportamentos
- **SEMPRE**: Checklist de ações obrigatórias
- **NUNCA**: Restrições absolutas
- Seja específico e acionável

## 💡 Fluxos de Trabalho

### Criar Nova Skill (Manual)
```
Usuário: Crie uma skill chamada python-expert
Assistente: [cria com template]

Usuário: Mostre a skill
Assistente: [mostra conteúdo]

Usuário: Salve edições com file_write
Assistente: [atualiza arquivo]

Usuário: Valide a skill
Assistente: ✅ Válida!
```

### Importar da Internet ⭐
```
Usuário: Importe https://doc.rust-lang.org/book/ como skill rust-book
Assistente: 
📥 Baixando...
🔄 Convertendo HTML → Markdown...
✅ Skill 'rust-book' importada!
```

### Gerenciar Skills
```
Usuário: Liste minhas skills
Assistente: [lista todas]

Usuário: Valide todas as skills
Assistente: [mostra válidas e inválidas]

Usuário: Remova a skill antiga
Assistente: [remove com backup]
```

## 🌟 Funcionalidades Avançadas

### Importação de URL
- Suporta Markdown e HTML
- Converte automaticamente para SKILL.md
- Extrai metadados (título, descrição, keywords)
- Validação obrigatória

### Extração de Keywords
- Análise automática de conteúdo
- Exclui palavras comuns
- Top 7 palavras mais relevantes

### Conversão HTML → Markdown
- Extrai conteúdo principal
- Converte headers, listas, formatação
- Preserva links e código

## 📚 Documentação

- `docs/SKILL_MANAGEMENT.md` - Guia completo de gerenciamento
- `docs/SKILL_IMPORT.md` - Guia de importação de URLs
- `skills/skill-manager/skill.md` - Skill de exemplo

## ✅ Exemplos Práticos

### Exemplo 1: Criar Skill para Python
```
Usuário: Crie uma skill para me ajudar com Python
Assistente: ✅ Skill 'python-helper' criada

Usuário: Edite para adicionar mais exemplos
Assistente: [mostra conteúdo atual]

Usuário: Agora me explique list comprehensions
Assistente: [usa skill python-helper automaticamente]
```

### Exemplo 2: Importar Documentação
```
Usuário: Importe https://raw.githubusercontent.com/user/guide.md como skill docker-guide
Assistente: ✅ Skill importada com 15 keywords

Usuário: Como criar um container?
Assistente: [responde baseado na documentação importada]
```

### Exemplo 3: Organizar Skills
```
Usuário: Liste skills
Assistente: 
- general: Assistente generalista
- python-helper: Especialista em Python
- rust-book: The Rust Programming Language
- docker-guide: Docker documentation

Usuário: Renomeie python-helper para python-expert
Assistente: ✅ Renomeado!
```

## 🔧 Comandos Úteis

```bash
# Ver skills disponíveis
"Liste suas skills"

# Criar skill
"Crie uma skill chamada meu-assistente"

# Validar
"Valide a skill meu-assistente"

# Importar
"Importe https://example.com/doc.md como skill exemplo"

# Editar
"Mostre a skill meu-assistente para eu editar"

# Remover
"Remova a skill meu-assistente"
```

## 🎓 Dicas

1. **Comece com skill-manager**: "Ative a skill skill-manager" para ajuda especializada
2. **Valide sempre**: Após criar/modificar, valide a skill
3. **Teste**: Faça perguntas relacionadas para ver se a skill funciona
4. **Importe**: Use URLs de documentação oficial para skills ricas
5. **Organize**: Use nomes descritivos em kebab-case

## 🚧 Limitações

- Nome da skill deve ser kebab-case
- Não pode remover a skill 'general'
- URLs com JavaScript pesado podem não funcionar bem
- Skills são locais (não sincronizadas entre instâncias)

---

**Pronto para usar!** Agora você pode criar, importar, organizar e gerenciar skills de forma completa! 🎉

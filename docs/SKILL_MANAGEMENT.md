# Gerenciamento de Skills

O RustClaw possui um sistema completo de gerenciamento de skills através de ferramentas integradas.

## Ferramentas Disponíveis

### 1. `skill_list`
Lista todas as skills disponíveis com suas descrições.

**Uso:**
```json
{}
```

**Exemplo:**
- "Quais skills estão disponíveis?"
- "Liste minhas skills"

### 2. `skill_create`
Cria uma nova skill a partir de template.

**Uso:**
```json
{ "name": "minha-skill" }
```

Ou com conteúdo customizado:
```json
{ 
  "name": "minha-skill",
  "custom_content": "# Skill: ..."
}
```

**Exemplo:**
- "Crie uma skill chamada python-expert"
- "Crie uma skill para me ajudar com programação"

### 3. `skill_edit`
Mostra o conteúdo atual de uma skill para edição.

**Uso:**
```json
{ "name": "minha-skill" }
```

**Exemplo:**
- "Mostre a skill python-expert"
- "Quero editar a skill coder"

Após visualizar, use `file_write` para salvar modificações.

### 4. `skill_validate`
Valida a sintaxe de uma ou todas as skills.

**Uso para uma skill:**
```json
{ "name": "minha-skill" }
```

**Uso para todas:**
```json
{}
```

**Exemplo:**
- "Valide a skill python-expert"
- "Verifique se todas as skills estão corretas"

### 5. `skill_rename`
Renomeia uma skill existente.

**Uso:**
```json
{ 
  "old_name": "nome-antigo",
  "new_name": "nome-novo" 
}
```

**Exemplo:**
- "Renomeie a skill coder para programmer"

### 6. `skill_delete`
Remove uma skill existente.

**Uso:**
```json
{ 
  "name": "minha-skill",
  "confirm": true 
}
```

**Exemplo:**
- "Remova a skill antiga"
- "Delete a skill teste"

⚠️ **A skill 'general' não pode ser removida.**

## Fluxo de Trabalho

### Criar Nova Skill

1. **Criar com template:**
   ```
   Usuário: Crie uma skill chamada "code-reviewer"
   ```

2. **Editar conteúdo:**
   ```
   Usuário: Mostre a skill code-reviewer
   Assistente: [mostra conteúdo]
   Usuário: Altere a descrição para "Especialista em revisão de código"
   ```

3. **Validar:**
   ```
   Usuário: Valide a skill code-reviewer
   ```

### Editar Skill Existente

1. **Visualizar:**
   ```
   Usuário: Mostre a skill coder
   ```

2. **Modificar via file_write:**
   ```
   Usuário: Salve na skills/coder/skill.md com [novo conteúdo]
   ```

3. **Validar:**
   ```
   Usuário: Valide a skill coder
   ```

## Estrutura do SKILL.md

```markdown
# Skill: [nome-da-skill]

## Descrição
[Breve descrição do propósito da skill]

## Contexto
[Contexto detalhado que o assistente usará quando esta skill estiver ativa]

## Keywords
- [palavra-chave1]
- [palavra-chave2]
- [palavra-chave3]

## Comportamento

### SEMPRE
- [Comportamento obrigatório 1]
- [Comportamento obrigatório 2]

### NUNCA
- [Comportamento proibido 1]
- [Comportamento proibido 2]

## Ferramentas Prioritárias
1. [tool_name1]
2. [tool_name2]

## Exemplos

### Input: "[exemplo de pergunta do usuário]"
**Bom:** [resposta desejada]
**Ruim:** [resposta a ser evitada]
```

## Convenções de Nomenclatura

- Use **kebab-case** (minúsculas com hífen)
- Exemplos válidos: `code-reviewer`, `python-expert`, `meeting-assistant`
- Evite: espaços, CamelCase, snake_case, caracteres especiais

## Detecção Automática

O sistema detecta automaticamente qual skill usar baseado em:
1. **Keywords** na mensagem do usuário
2. **Contexto** da conversa
3. **Skill atualmente ativa** (evita mudanças frequentes)

A skill é persistida por chat no banco de dados.

## Hot Reload

As skills são recarregadas automaticamente quando:
- Arquivos SKILL.md são modificados
- Novas skills são adicionadas
- Skills são removidas

Não é necessário reiniciar o RustClaw!

## Dicas

1. **Comece com a skill manager:**
   - "Ative a skill skill-manager"
   - Isso ajuda o assistente a entender melhor o contexto

2. **Valide sempre:**
   - Após criar ou modificar uma skill, valide-a
   - Use `skill_validate` sem parâmetros para validar todas

3. **Use exemplos concretos:**
   - Inclua exemplos de Input/Bom/Ruim na skill
   - Isso ajuda o assistente a entender o comportamento esperado

4. **Teste a skill:**
   - Após criar, teste com perguntas relacionadas
   - Ajuste keywords se necessário

## Exemplo Completo

```
Usuário: Crie uma skill para me ajudar com análise de dados em Python
Assistente: [cria skill data-analyst]

Usuário: Valide a skill
Assistente: [valida e mostra informações]

Usuário: Agora use essa skill para me ajudar com pandas
Assistente: [automaticamente ativa data-analyst e ajuda]
```

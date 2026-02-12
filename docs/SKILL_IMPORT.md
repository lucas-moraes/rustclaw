# Importar Skills de URLs

A ferramenta `skill_import_from_url` permite importar e converter documentação de qualquer URL em uma skill do RustClaw.

## Funcionalidades

✅ **Download de qualquer URL** - Suporta HTTP/HTTPS  
✅ **Detecção automática** - Identifica se é HTML ou Markdown  
✅ **Conversão HTML → Markdown** - Extrai conteúdo principal de páginas HTML  
✅ **Conversão automática** - Converte qualquer documentação em formato SKILL.md  
✅ **Extração de metadados** - Título, descrição e keywords automáticas  
✅ **Validação obrigatória** - Sempre valida antes de finalizar  
✅ **Rollback automático** - Remove arquivos em caso de erro

## Uso Básico

### Importar Documentação Markdown

```
Usuário: Importe https://raw.githubusercontent.com/user/repo/main/guide.md como skill python-guide
```

### Importar Página HTML

```
Usuário: Importe https://docs.python.org/3/tutorial/ como skill python-tutorial
```

### Importar README do GitHub

```
Usuário: Importe https://github.com/user/project/blob/main/README.md como skill projeto-helper
```

## Parâmetros

```json
{
  "url": "https://example.com/doc.md",
  "skill_name": "minha-skill"
}
```

- **url** (obrigatório): URL da documentação a ser importada
- **skill_name** (obrigatório): Nome da skill a ser criada (kebab-case)

## Fluxo de Conversão

1. **Download**: Baixa o conteúdo da URL
2. **Detecção**: Identifica se é HTML ou Markdown
3. **Extração** (se HTML): Converte HTML para Markdown
4. **Conversão**: Transforma em formato SKILL.md
5. **Criação**: Salva no diretório `skills/<nome>/skill.md`
6. **Validação**: Valida a sintaxe SKILL.md
7. **Resultado**: Retorna sucesso ou erro com rollback

## Conversão Automática

### Se o conteúdo já for SKILL.md válido
→ Usa diretamente sem modificações

### Se for Markdown genérico ou HTML
→ Converte automaticamente:

```markdown
# Título Original
→ # Skill: nome-da-skill

Primeiro parágrafo
→ ## Descrição
    Primeiro parágrafo...

Conteúdo completo
→ ## Contexto
    Contexto baseado em documentação importada de [domínio]
    
    Conteúdo completo...

Palavras relevantes extraídas
→ ## Keywords
    - keyword1
    - keyword2
    - ...
```

### Metadados Extraídos

**Título**: Primeiro H1 encontrado  
**Descrição**: Primeiro parágrafo após o título (limitado a 200 caracteres)  
**Keywords**: Top 7 palavras mais relevantes extraídas do conteúdo (excluindo palavras comuns)  
**Domínio**: Extraído da URL para referência

## Exemplos de Uso

### Exemplo 1: Importar Guia de Rust

```
Usuário: Importe https://doc.rust-lang.org/book/ como skill rust-book

Assistente: 
📥 Baixando conteúdo de https://doc.rust-lang.org/book/...
✅ Conteúdo baixado (15000 bytes)
🔄 Convertendo HTML para Markdown...
🔄 Convertendo para formato SKILL.md...
🔍 Validando skill...

✅ Skill 'rust-book' importada com sucesso!

📊 Detalhes:
- Nome: rust-book
- Descrição: The Rust Programming Language...  
- Keywords: rust, programming, language, tutorial, book, documentation, guide
- Arquivo: skills/rust-book/skill.md
```

### Exemplo 2: Importar README

```
Usuário: Importe https://raw.githubusercontent.com/serde-rs/serde/master/README.md como skill serde-guide

Assistente:
✅ Skill 'serde-guide' importada com sucesso!

📊 Detalhes:
- Nome: serde-guide
- Descrição: Serde is a framework for serializing and deserializing Rust data structures...
- Keywords: serde, serialization, deserialization, json, framework
- Arquivo: skills/serde-guide/skill.md
```

### Exemplo 3: Erro - Skill Já Existe

```
Usuário: Importe https://example.com/doc.md como skill python

Assistente:
❌ Erro: Skill 'python' já existe. Use outro nome ou remova a existente primeiro.
```

## Extração de Conteúdo HTML

A ferramenta tenta extrair o conteúdo principal das páginas HTML na seguinte ordem:

1. `article`
2. `main`
3. `[role='main']`
4. `.content`
5. `.documentation`
6. `.markdown-body`
7. `.readme`
8. `#content`
9. `#readme`
10. `body` (fallback)

Elementos suportados na conversão:
- Headers (h1-h4) → Markdown headers
- Parágrafos → Texto com quebras de linha
- Listas (ul/ol) → Listas Markdown
- Negrito/itálico → `**texto**` / `*texto*`
- Código inline → `` `código` ``
- Blocos de código → ```código```
- Links → `[texto](url)`

## Regras de Validação

- Nome não pode estar vazio
- Nome não pode conter espaços, `/`, `\`
- Nome não pode ser "general" (skill protegida)
- URL deve ser acessível
- Conteúdo deve ser parseável
- Sintaxe SKILL.md deve ser válida

## Dicas

### URLs Recomendadas

✅ **Bom**:
- Arquivos raw do GitHub (`raw.githubusercontent.com`)
- Documentação em Markdown direto
- Páginas de documentação simples

⚠️ **Pode precisar de ajustes**:
- Páginas com muito JavaScript (use URLs diretas)
- Documentação com navegação complexa
- Páginas com muitos anúncios/popups

### Melhores Práticas

1. **Prefira URLs raw**: Use `raw.githubusercontent.com` ao invés de `github.com`
2. **Verifique o resultado**: Após importar, valide a skill
3. **Edite se necessário**: Use `skill_edit` + `file_write` para ajustar
4. **Teste a skill**: Faça perguntas relacionadas para ver se funciona bem

### Exemplo Completo

```
Usuário: Importe https://raw.githubusercontent.com/rust-lang/rust-by-example/master/README.md como skill rust-examples

Assistente: [importa e converte]

Usuário: Valide a skill rust-examples
Assistente: ✅ Skill válida!

Usuário: Mostre a skill
Assistente: [mostra conteúdo]

Usuário: Agora me ajude com lifetimes em Rust
Assistente: [usa a skill rust-examples automaticamente]
```

## Troubleshooting

### "Erro ao acessar URL"
→ Verifique se a URL está correta e acessível  
→ Alguns sites bloqueiam bots (use URLs raw quando possível)

### "Não foi possível extrair conteúdo do HTML"
→ A página pode ser muito complexa ou dinâmica  
→ Tente acessar diretamente via `http_get` e depois criar manualmente

### "Skill criada mas com erro de validação"
→ O conteúdo foi removido automaticamente  
→ Verifique o formato SKILL.md e tente novamente

### "Skill já existe"
→ Use outro nome ou remova a existente primeiro com `skill_delete`

## Comparação com Outras Ferramentas

| Ferramenta | Uso | Quando Usar |
|------------|-----|-------------|
| `skill_import_from_url` | Importar de URL | Quando tem documentação online |
| `skill_create` | Criar do zero | Quando vai escrever customizado |
| `skill_edit` + `file_write` | Editar existente | Quando precisa ajustar |
| `http_get` + manual | Baixar e criar | Quando precisa de controle total |

A ferramenta `skill_import_from_url` automatiza o fluxo completo: download → conversão → formatação → validação → salvamento!

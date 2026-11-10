# GitPass · Família Rota

A família usa nós e curvas do painel de detalhes para ligar a iconografia de repositórios, contas e Git. Foram desenhados **43 ícones de interface** e o símbolo opcional **Rota**.

## Abra primeiro

- **Catalogo.html** permite buscar ícones, alternar entre claro e escuro, testar de 12 a 32 px e baixar cada SVG.
- **GitPass_Icones_no_App.html** aplica os ícones à prévia interativa do GitPass. Clique em `orbit-engine` para ver os glifos no painel sobreposto e nas abas. A prévia usa os dados de demonstração já existentes.
- **Prancha_GitPass_Icones.svg** e **Prancha_GitPass_Icones.png** mostram a direção visual, a aplicação no painel e uma seleção em tamanhos reais.

## Regras de aplicação

| Contexto | Tamanho inicial | Tratamento |
|---|---:|---|
| Sidebar | 18 px | Traço neutro nos itens comuns e cor do tema no item ativo. |
| Botão com texto | 16 px | Ícone e texto herdam a mesma cor. |
| Badge de estado | 12–14 px | Use silhuetas simples como `check`, `alert` ou `lock`, acompanhadas do texto. |
| Ícone de seção ou destaque no painel | 20–24 px | Realce um nó com a cor do perfil em um ponto de destaque. |

Todos os SVGs usam grade `24 × 24`, traço `1.8` e `currentColor`. Use o realce de nó em seleção ou destaque visual. A cor de alerta ou de confirmação vem do estado e continua acompanhada por um rótulo. Em botões que só têm um ícone, adicione `aria-label` ao botão. O SVG de UI exportado por `icon()` é decorativo.

## Integração com Cursor

1. Abra a raiz do GitPass no Cursor. Compare as chaves do `gitpass-desktop/src/lib/icons.ts` atual com as do arquivo `icons.ts` do kit. O kit mantém a assinatura `icon(nome, tamanho)`, o tipo `IconName` e o `brandSymbol()` do commit `49cecf9`. Se o checkout tiver ícones mais novos, preserve suas chaves ao mesclar.
2. Copie ou mescle `icons.ts` no caminho acima. Comece pela sidebar, pelo painel de repositório e pelos estados. Preserve os tamanhos, rótulos, handlers e navegação existentes.
3. Se quiser ativar a cor secundária, adicione `gitpass-icons.css` aos estilos da aplicação e defina `--gitpass-icon-accent` só em contextos selecionados. Exemplos para a paleta da prévia:

```css
.nav-item[aria-current="page"] {
  --gitpass-icon-accent: #136e70;
}

:root[data-theme="dark"] .nav-item[aria-current="page"] {
  --gitpass-icon-accent: #98eae1;
}

@media (prefers-color-scheme: dark) {
  :root:not([data-theme]) .nav-item[aria-current="page"] {
    --gitpass-icon-accent: #98eae1;
  }
}
```

4. Verifique os ícones em ambos os temas e rode `npm run build` em `gitpass-desktop/`. No macOS, confira o resultado no WebView do app com `npm run tauri dev`.

O `brandSymbol()` mantém o símbolo já usado no app. O arquivo `svg/brand-route.svg` fica disponível para avaliar a nova marca separadamente.

## Mapeamento para os novos fluxos

| Fluxo | Chaves de ícone |
|---|---|
| Diagnóstico de commit e push | `diagnostic`, `commit`, `push` |
| Regras por pasta e organização | `folder`, `link`, `filter` |
| Radar de trabalho | `activity`, `branch`, `alert` |
| Paleta de comandos | `command`, `search`, `terminal`, `editor` |
| Histórico e revisão de autoria | `history`, `undo`, `commit` |
| Caixa de entrada de PRs | `pull-request`, `refresh`, `external` |

## Outros usos dos SVGs

A pasta `svg/` tem um arquivo por glifo. O `gitpass-icons.sprite.svg` também agrupa todos os símbolos com prefixo `gp-`. Para usar o sprite, inclua os símbolos no DOM da interface uma vez e referencie o ID correspondente.

```html
<svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true">
  <use href="#gp-repo"></use>
</svg>
```

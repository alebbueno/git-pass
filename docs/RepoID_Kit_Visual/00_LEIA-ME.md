# RepoID

RepoID é o nome provisório usado nesta identidade para um app macOS que associa perfis Git a repositórios. Na marca, o nó inferior representa o projeto e as duas pontas representam identidades que podem ser associadas a ele.

## Arquivos

| Arquivo | Uso |
| --- | --- |
| Icone/RepoID-1024.png | Arte 1024 × 1024 com transparência para o ícone e materiais do projeto. |
| Icone/RepoID.icns | Ícone estático multirresolução para empacotamento no macOS. |
| Logo/RepoID-logo-fundo-claro.svg | Marca horizontal para superfícies claras. |
| Logo/RepoID-logo-fundo-escuro.svg | Marca horizontal para superfícies escuras. |
| Logo/RepoID-logo-monocromatico.svg | Marca horizontal em uma cor. |
| Logo/RepoID-simbolo.svg | Bifurcação em uma cor para a barra lateral ou como camada em um ícone. |
| Logo/*.png | Exportações transparentes das duas versões coloridas. |
| Paleta/repoid-tokens.json | Tokens sRGB de marca, superfícies, texto, seleção, perfis e estados. |
| Paleta/repoid-tokens.css | Os mesmos tokens como variáveis CSS com tema do sistema e opção de seleção manual. |
| RepoID_Identidade_Visual.png | Prancha com marca, ícone, cores e uma amostra de interface. |

Os SVGs trazem o lettering em curvas. Use a família de sistema na interface e a monoespaçada do sistema para nomes de repositório, branches e caminhos.

## Cores e materiais

- O índigo aciona controles no modo claro. A íris cumpre o mesmo papel no modo escuro.
- Cada perfil recebe uma família de cor persistente. No exemplo, Trabalho recebe lavanda e Pessoal recebe aqua. Mantenha o nome e as iniciais visíveis. Use a mesma família de cor nas duas aparências.
- Use vidro na navegação, na barra de ferramentas e no seletor de perfis. Listas, diffs e configurações usam superfícies opacas. Os tokens de material incluem uma cor sólida para a opção de reduzir transparência.
- Use cores de feedback para estados como vínculo confirmado, revisão necessária e erro. Inclua um rótulo ou símbolo de estado.
- Para integrar um ícone dinâmico no Xcode, use o SVG do símbolo como ponto de partida no Icon Composer e configure o material e o fundo em camadas. O ICNS do kit é uma exportação estática.

## Contraste dos pares sólidos

| Par | Claro | Escuro |
| --- | --- | --- |
| Texto principal / fundo | 16,16:1 | 17,22:1 |
| Texto secundário / painel | 7,09:1 | 8,10:1 |
| Texto do botão / cor de ação | 6,55:1 | 8,29:1 |
| Texto selecionado / seleção | 9,29:1 | 11,15:1 |

As combinações de texto e fundo dos perfis e estados no arquivo de tokens atingem pelo menos 6,25:1 entre os dois temas. Avalie a composição das superfícies translúcidas quando o app estiver renderizado.

## Referências

- [Materiais nas Human Interface Guidelines](https://developer.apple.com/design/human-interface-guidelines/materials)
- [Icon Composer](https://developer.apple.com/icon-composer/)


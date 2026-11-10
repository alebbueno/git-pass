# GitPass · Splash Rota

A abertura desenha a trilha do símbolo Rota, acende os nós e revela o nome GitPass. Em uma abertura normal, com a interface já montada, a sequência termina em cerca de 1,6 s. A splash usa o mesmo fundo escuro de partida da janela atual.

## Veja a animação

- **GitPass_Splash_Preview.html** abre no navegador e passa da splash para a prévia interativa do app. Use **Repetir abertura** no topo para rever.
- **GitPass_Splash_Motion.mp4** mostra o estudo de movimento da arte vetorial.
- **Splash_Rota.svg** é o desenho editável. **Splash_Rota.png** mostra a composição em repouso.

## Aplicação no app

O patch foi preparado para o GitPass no commit `49cecf9`. Ele toca `gitpass-desktop/index.html`, que já contém a tela de carregamento, e `gitpass-desktop/src/main.ts`, que monta a primeira tela.

1. Extraia o ZIP e abra o repositório GitPass no Cursor.
2. Na raiz do repositório, verifique e aplique o patch. Troque o caminho pelo local em que você extraiu o arquivo.

```bash
git apply --check "/caminho/GitPass_Splash.patch"
git apply "/caminho/GitPass_Splash.patch"
```

3. No macOS, execute `npm run tauri dev` a partir de `gitpass-desktop/`. Encerre o app e abra de novo para testar uma inicialização completa. Ao ocultar e mostrar a mesma janela, a splash já encerrada não é recriada.

Se o patch não encaixar porque o checkout mudou, use este prompt no Cursor.

> Leia `GitPass_Splash.patch` e aplique os trechos correspondentes no checkout atual. Use o HTML e os estilos da splash em `gitpass-desktop/index.html`, antes de `#app`. Ligue a saída à montagem da primeira tela em `src/main.ts`. Preserve o tema e a navegação da versão atual e adapte apenas os trechos necessários. Rode o build e valide a abertura no macOS.

## Comportamento

- A arte usa SVG e CSS locais e aparece antes de o bundle montar a interface.
- A saída acompanha a primeira tela montada. Carregamentos da integração, da sidebar e da rede continuam em seus fluxos normais.
- A cobertura tem um limite de aproximadamente 2,8 s se o frontend demorar. Nesse caso, a área de preparação ou a interface carregada fica visível.
- Com **Reduzir movimento**, a arte fica estática durante o preparo e sai assim que a primeira tela estiver pronta.

O patch foi conferido contra o checkout de referência. A abertura na janela nativa deve ser conferida no macOS.

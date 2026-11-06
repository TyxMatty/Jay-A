# Jay-A

App de áudio da linha Jay: filtros do microfone (uma opção no [Jay-C](https://github.com/TyxMatty/JayC), no lugar do filtro dele), edição de áudio, soundboard e conversão de formatos. Windows primeiro.

Arquitetura, fluxos e etapas: [`docs/ARQUITETURA.md`](docs/ARQUITETURA.md). Hoje o app está na **etapa 0**: a janela (nos estilos Calmo, Neon e Suave do Jay-C) e o ícone na bandeja. As seções de áudio ainda são só a casca.

## Rodando

Precisa das mesmas ferramentas do Jay-C (Node 22+, Rust com MSVC; ver o `docs/CONTINUAR.md` dele).

```bash
npm install
npm run dev
```

`npm run dev` abre a janela do Jay-A (Tauri) com recarregamento automático. Fechar a janela deixa o Jay-A na bandeja; para sair, clique com o botão direito no ícone dele. `jay-a.exe --background` abre só na bandeja (é assim que o Jay-C vai iniciá-lo). Para abrir só a interface no navegador, sem Tauri: `npm run web` (porta 5175).

O build do Rust gera vários GB. Para não ir para o OneDrive, crie um `.cargo/config.toml` (fica fora do git) apontando para outra pasta:

```toml
[build]
target-dir = "C:/Users/<você>/.cargo-target/jay-a"
```

## Testes

```bash
npm test
```

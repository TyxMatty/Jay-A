# Jay-A

[English](README.md) · **Português**

App de áudio da linha Jay: filtros do microfone (uma opção no [Jay-C](https://github.com/TyxMatty/Jay-C), no lugar do filtro dele), edição de áudio, soundboard e conversão de formatos. Windows primeiro.

É para quem faz chamadas e lives pelo Jay-C e quer a voz limpa (sem teclado, cliques ou vozes ao fundo) e ferramentas de áudio simples num app leve, sem assinar um serviço de remoção de ruído à parte.

O que está pronto e o que vem a seguir: [`ROADMAP.md`](ROADMAP.md) (em inglês). Arquitetura, fluxos e etapas: [`docs/ARQUITETURA.md`](docs/ARQUITETURA.md). Pronto até aqui: a janela (nos estilos Calmo, Neon e Suave do Jay-C) e o ícone na bandeja; em Filtros, o microfone com a limpeza do WebRTC, o medidor e o "Ouvir meu microfone", a cadeia de efeitos (remoção de ruído por IA com DeepFilterNet ou RNNoise, filtro, gate, equalizador, compressor, de-esser, limitador) e os presets; e o Jay Link, que entrega o microfone filtrado ao Jay-C nas chamadas, com cancelamento de eco. O soundboard, o editor e o conversor ainda são só a casca.

## Rodando

Precisa das mesmas ferramentas do Jay-C (Node 22+, Rust com MSVC e Python com Meson e Ninja, para o processamento de áudio do WebRTC; ver o `docs/CONTINUAR.md` dele).

```bash
npm install
npm run dev
```

`npm run dev` abre a janela do Jay-A (Tauri) com recarregamento automático. Fechar a janela deixa o Jay-A na bandeja; para sair, clique com o botão direito no ícone dele. `jay-a.exe --background` abre só na bandeja (é assim que o Jay-C o inicia). Para abrir só a interface no navegador, sem Tauri: `npm run web` (porta 5175).

O build do Rust gera vários GB. Para não ir para o OneDrive, crie um `.cargo/config.toml` (fica fora do git) apontando para outra pasta:

```toml
[build]
target-dir = "C:/Users/<você>/.cargo-target/jay-a"
```

## Testes

```bash
npm test
cd crates/jay-audio && cargo test
```

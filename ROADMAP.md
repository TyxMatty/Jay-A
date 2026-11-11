# Roteiro do Jay-A

**Português** · [English](ROADMAP.en.md)

O Jay-A é o app de áudio da linha Jay: filtros do microfone para as chamadas do [Jay-C](https://github.com/TyxMatty/Jay-C) e para outros apps, soundboard, editor de áudio e conversão de formatos. Windows primeiro.

Cada etapa termina com algo testável. O plano completo, com o "pronto quando" e as medições de cada etapa terminada, está em [`docs/ARQUITETURA.md`](docs/ARQUITETURA.md), seção 12.

## Pronto

- [x] **0. Scaffold:** Tauri v2 + Vite + JS puro, os três estilos do Jay-C (Calmo, Neon, Suave), ícone na bandeja, instância única e `--background`.
- [x] **1. Engine de áudio:** captura do microfone por WASAPI com o AudioProcessing do WebRTC (cancelamento de eco, supressão de ruído, ganho automático), o "Ouvir meu microfone" e o medidor.
- [x] **2a. Motor da cadeia de efeitos:** parâmetros com faixa e escala, um rack reordenável trocado sem lock, crossfade de 10 ms, presets em JSON e zero alocações na thread de áudio.
- [x] **2b. Efeitos:** filtro, gate (com histerese e hold), equalizador de 10 bandas, compressor, de-esser e limitador, mais os 4 presets do app. A cadeia inteira usa cerca de 0,5% de um núcleo.
- [x] **2c. Aba Filtros:** montar a cadeia com mouse ou teclado, controles gerados da descrição de cada parâmetro, medidores por efeito, latência total e presets (salvar, importar, exportar).
- [x] **2d. Remoção de ruído por IA:** DeepFilterNet 3 e RNNoise como efeitos da cadeia, contra teclado, cliques e vozes ao fundo.
- [x] **3. Servidor do Jay Link:** o microfone filtrado vai ao Jay-C por um named pipe local que só o mesmo usuário abre, a cada 20 ms (p99 de 20,4 ms no app instalado), com o áudio da chamada do Jay-C como referência de eco. As threads de áudio rodam com a prioridade "Pro Audio" do Windows, para a CPU ocupada não travá-las.
- [x] **4. Soundboard:** clipes importados de qualquer formato comum, tocados por atalhos globais com outro app em foco, mixados na chamada e nos fones, com "parar tudo" e os clipes tirados do eco.
- [x] **Microfone para outros apps:** o microfone filtrado e o soundboard chegam ao Discord, ao OBS e a outros pelo VB-CABLE gratuito, que o Jay-A baixa, confere e instala pelo instalador da própria VB-Audio.
- [x] **5. Conversão de formatos:** conversão em lote para WAV, FLAC, MP3, OGG e Opus, uma tarefa por núcleo; um arquivo corrompido falha sozinho e os outros saem.
- [x] **6. Editor de áudio:** cortar, fade, ganho, normalizar, inverter e aparar silêncio, com desfazer, prévia nos fones e exportação em qualquer formato da conversão.
- [x] **Instalador e atualizações:** instalador NSIS por usuário (sem administrador) nos Releases do GitHub, dentro do setup do Jay-I; instalado e atualizado pelo hub [Jay-H](https://github.com/TyxMatty/Jay-H) e pelo próprio app (uma pílula avisa da versão nova).
- [x] **Tour de boas-vindas e docs em inglês.**

## A seguir

- [ ] **Jay Link numa chamada de verdade** (com a I5 do Jay-C): o eco removido, fechar o Jay-A no meio da chamada não corta a voz (o filtro do Jay-C assume) e o som de outros apps não fica abafado.
- [ ] **Editor, o resto:** velocidade, normalização por loudness (LUFS) e aplicar um preset de filtros num arquivo.
- [ ] **7. Efeitos de voz:** tom, robô e reverb como efeitos da cadeia, com latência e CPU medidas.
- [ ] **8. Soundboard dentro do Jay-C:** um botão na barra da chamada que lista e toca os clipes do Jay-A.
- [ ] **9. Release com o Jay-C:** o Jay-C oferece o Jay-A; os dois instalados numa máquina limpa fazem uma chamada filtrada, e o Jay-C sozinho continua funcionando com o filtro dele.

## Decisões em aberto

- Soundboard com o microfone mutado no Jay-C: mutar tudo (simples) ou uma faixa própria do soundboard no servidor.
- Plugins de terceiros (VST3/CLAP) na cadeia, cada um no seu processo: depois da etapa 7.

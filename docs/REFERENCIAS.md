# Jay-A — Estudo do EasyEffects e do Carla

**Português** · [English](REFERENCIAS.en.md)

Notas da leitura do código de dois projetos open-source de áudio, feita a pedido do usuário antes de escrever os filtros do Jay-A. Os dois são GPL e o Jay-A é **MIT** (decisão D6): daqui saem ideias, organização e faixas de parâmetros, **nunca código**. Os caminhos citados são dos repositórios deles.

Lido em 2026-11-06: EasyEffects `011bdc5` (2026-11-05), Carla `97a9e07` (2026-03-19).

## 1. EasyEffects (GPL-3.0)

App de efeitos para o PipeWire, no Linux: entrada (microfone) e saída (alto-falante). Os efeitos em si vêm de bibliotecas e plugins (LSP, Calf, RNNoise, DeepFilterNet, SpeexDSP, SoundTouch, libebur128, WebRTC).

### 1.1 A cadeia

- **Uma lista ordenada de instâncias** (`plugins` no `easyeffects_db_streaminputs.kcfg`; `plugins_order` no preset), com nomes como `gate#0` e `compressor#1`. A pessoa escolhe a ordem e pode repetir um efeito. Começa **vazia**.
- **Cada efeito é um nó** com entrada e saída estéreo, mais uma entrada "probe" opcional para sidechain (`src/plugin_base.hpp`). O cancelador de eco usa essa entrada como referência: é o que toca no alto-falante.
- **Controles que todo efeito tem:** `bypass`, ganho de entrada e ganho de saída (−36 a +36 dB), e medidores de pico na entrada e na saída de cada efeito (`get_peaks`, `apply_gain` em `src/plugin_base.cpp`).
- **Latência:** cada efeito informa a sua (`get_latency_seconds`), e a soma da cadeia aparece na tela (`src/effects_base.cpp`).
- **Medidor e espectro** ficam no fim da cadeia, como nós à parte (`output_level`, `spectrum`).

### 1.2 Presets

JSON com uma seção por cadeia (`src/presets_manager.cpp`, `src/*_preset.cpp`):

```json
{ "input": { "blocklist": [], "plugins_order": ["gate#0", "compressor#0"],
             "gate#0": { "bypass": false, "input-gain": 0, "output-gain": 0, "attack": 20, "release": 100 } } }
```

Também tem presets da comunidade, carregamento automático de preset por dispositivo, uma pasta própria de impulsos (convolver) e modelos do RNNoise.

### 1.3 Faixas de parâmetros (dos `src/contents/kcfg/*.kcfg`)

Faixas e padrões que valem como ponto de partida para a voz no Jay-A:

| Efeito | Parâmetros (padrão [mín..máx]) |
|---|---|
| Gate | attack 20 ms [0..2000], release 100 ms [0..5000], limiar −24 dB [−60..0] com zona −6 dB (joelho), **histerese** opcional (limiar de fechar separado do de abrir), redução −24 dB, makeup; detecção Peak/RMS; filtros no sidechain (HPF/LPF) |
| Compressor | limiar −12 dB [−60..0], ratio 4 [1..100], joelho −6 dB [−24..0], attack 20 ms, release 100 ms, makeup [−60..60]; modos para baixo/para cima/boost; dry/wet |
| Expander | igual ao compressor, modo para cima por padrão |
| Limitador | limiar 0 dB [−48..0], **lookahead 5 ms** [0,1..20], attack 5 ms, release 5 ms, oversampling e "true peak" opcionais, nivelamento automático (ALR) |
| De-esser | limiar −18 dB, ratio 3 [1..20], faixa de detecção em 6 kHz / 4,5 kHz, makeup [0..36] |
| Filtro | tipos: passa-baixas, passa-altas, shelf, bell, passa-banda, notch, ressonância, all-pass; frequência [10..24000], inclinação x1..x16, ganho [−36..36]; IIR por padrão (FIR/FFT opcionais) |
| Equalizador | até **32 bandas** [1..32], cada uma com tipo, frequência, ganho [−36..36], Q e inclinação; IIR por padrão |
| RNNoise | wet [−100..20] dB, **VAD** opcional com limiar 50% e **release** de 20 ms (segura a voz aberta depois que o VAD cai), modelos trocáveis |
| DeepFilterNet | **limite de atenuação** 100 dB [0..100], limiares de processamento ERB/DF, pós-filtro; também roda o modelo DPDFNet |
| Speex | denoise −70 dB, AGC, VAD e dereverb opcionais |
| Autogain | alvo −23 LUFS [−100..0], referência momentânea/curta/integrada (EBU R128, libebur128), histórico 15 s, limiar de silêncio −70 dB |
| Echo canceller | **WebRTC AudioProcessing**: AEC (com modo mobile), supressão de ruído Low..VeryHigh, passa-altas, AGC. Blocos de 10 ms (`src/echo_canceller.cpp`) |
| Pitch | semitons [−12..12], cents, oitavas; SoundTouch (LGPL) |

### 1.4 Detalhes que importam

- O **echo canceller é o mesmo motor que o Jay-A já usa** (WebRTC APM, 10 ms, `ProcessReverseStream` + `ProcessStream`), com a referência pela entrada probe. Confirma o desenho da seção 5 do `ARQUITETURA.md`.
- **RNNoise e DeepFilterNet trabalham em 48 kHz**; o EasyEffects reamostra quando o grafo roda em outra taxa. Como o Jay-A é todo em 48 kHz, não precisa.
- **O VAD com "release"** do RNNoise é a mesma ideia do *hold* do gate do Jay-C (`mic-shape.js`): segurar alguns blocos depois que a voz cai, para não cortar o fim das palavras.
- **A histerese do gate** (abrir num nível e fechar em outro, mais baixo) evita o gate ficar abrindo e fechando quando a voz está perto do limiar. O gate do Jay-C não tem.
- **O DeepFilterNet entra como plugin LADSPA** que o próprio projeto DeepFilterNet publica (`libDF`, em Rust). O Jay-A pode usar a biblioteca Rust direto.

## 2. Carla (GPL-2.0 ou posterior)

Host de plugins (LADSPA, DSSI, LV2, VST2, VST3, CLAP, AU, JSFX, SF2/SFZ), com drivers nativos (JACK, ALSA, CoreAudio, DirectSound/WASAPI via RtAudio).

### 2.1 Modos de processamento (`source/backend/CarlaBackend.h`, `EngineProcessMode`)

- **Rack** (`CONTINUOUS_RACK`): os plugins rodam **em série, na ordem**, sempre em estéreo. A saída de um é a entrada do próximo.
- **Patchbay:** um grafo livre, em que se liga qualquer saída a qualquer entrada.
- **Single/Multiple client** (JACK) e **Bridge** (o processo que roda um plugin isolado).

Para a voz, o rack resolve: é a mesma cadeia linear do EasyEffects. O patchbay é mais poder do que o Jay-A precisa.

### 2.2 O loop de tempo real do rack (`source/backend/engine/CarlaEngineGraph.cpp`, `RackGraph::process`)

- Para cada plugin, na ordem: se está desligado **ou se não consegue pegar o lock na hora (`tryLock`)**, ele é pulado neste bloco. **A thread de áudio nunca espera** por quem está mexendo no plugin (a interface, carregar estado).
- Entre plugins, os buffers são copiados (a saída vira a entrada).

### 2.3 Parâmetros (`source/backend/CarlaBackend.h`, `ParameterData`, `ParameterRanges`)

- Cada parâmetro tem **padrão, mínimo, máximo, passo, passo pequeno e passo grande**, mais um `fixValue` que corrige (clampa) qualquer valor recebido.
- Tem também dicas de tipo: **booleano, inteiro, logarítmico**, somente leitura, automatizável e não salvo.
- **Controles do host aplicados a qualquer plugin** (`PLUGIN_CAN_DRYWET`, `_VOLUME`, `_BALANCE`): dry/wet, volume e balanço ficam fora do plugin, num pós-processamento comum (`CarlaPluginBridge.cpp`, "Post-processing").

### 2.4 Plugins em outro processo (`source/backend/plugin/CarlaPluginBridge.cpp`)

- O plugin roda num **processo à parte**. O áudio vai e volta por **memória compartilhada**; os comandos de tempo real passam por um anel de opcodes com semáforo.
- O host escreve "processe N quadros" e **espera com timeout** (`waitForClient`). Se o plugin não responde, o bloco é marcado como perdido, o plugin para de ser processado, e a interface avisa: um plugin travado não derruba o host.
- O protocolo tem **versão** (`CARLA_PLUGIN_BRIDGE_API_VERSION_MINIMUM` 6, atual 10), conferida na conexão.

É o mesmo problema do Jay Link (outro processo, tempo real, travamento), resolvido com memória compartilhada em vez de pipe. A reserva quente do Jay-C, com os 60 ms sem bloco, faz o papel do timeout do Carla.

## 3. O que o Jay-A adota

| Ideia | De onde | Onde entra no Jay-A |
|---|---|---|
| Cadeia = lista ordenada de instâncias (`gate#0`), escolhida pelo usuário, começando de um preset | EasyEffects | O rack da etapa 2 |
| Todo efeito com liga/desliga (bypass), ganho de entrada e de saída, medidores de entrada e de saída, latência informada | EasyEffects + Carla (controles do host) | O `Effect` do engine: o host aplica isso de forma igual para todos |
| Parâmetros com padrão, mín., máx., passo e escala (linear, log, dB), clampados na entrada | Carla | O `ParamSpec` do engine e os sliders da UI |
| A thread de áudio nunca espera um lock: mudanças chegam por fila sem lock, e a cadeia nova entra inteira entre dois quadros | Carla (`tryLock`), adaptado | A regra de tempo real do engine |
| Preset em JSON, com a ordem e os parâmetros de cada instância | EasyEffects | `presets/*.json`; importar e exportar |
| Faixas e padrões para voz (seção 1.3) | EasyEffects | Os padrões dos efeitos da etapa 2 |
| Gate com histerese e hold | EasyEffects + Jay-C | O gate da etapa 2 |
| Limitador com lookahead de 5 ms | EasyEffects | O limitador do fim da cadeia |
| RNNoise com VAD e release; DeepFilterNet com limite de atenuação | EasyEffects | A supressão forte da etapa 7 (`nnnoiseless` BSD, `libDF` MIT/Apache) |
| Autogain por loudness (LUFS) | EasyEffects | O normalizar do editor e da conversão (`ebur128` MIT) |
| Processo separado com timeout e versão de protocolo | Carla | Já é o Jay Link; plugins de terceiros (D7) seguiriam o mesmo modelo |

**Não adota:** o patchbay (grafo livre), os plugins LV2/VST no v1 (D7), oversampling e dithering no limitador, os modos FIR/FFT dos filtros (a latência não vale para voz ao vivo), e o SoundTouch (LGPL; o candidato é o `signalsmith-stretch`, MIT).

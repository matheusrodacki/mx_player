# MX Player — Roadmap MVP

Tarefas organizadas por fase conforme o PRD. Marque com `[x]` ao concluir.

---

## Fase 1 · MVP CLI + Parser + Engine

> Objetivo: parser da lista de sources funcionando + libmpv abrindo stream pela linha de comando + lógica de fallback validada.

### 1.1 · Configuração de Ambiente

- [ ] **T01** — Resolver integração da `libmpv` no Windows: baixar `mpv-dev` (binários MinGW), configurar `MPV_LIB_PATH` e testar `cargo build` com a crate `libmpv-sys` ou `mpv`.
  - Investigar se `libmpv` v2 (`mpv >= 0.35`) roda em Win7 (depende de `msvcrt` vs `ucrt`).
  - Alternativa: crate `mpv` (wrapper seguro) vs `libmpv-sys` (bindings raw).
- [ ] **T02** — Confirmar versão máxima do tokio compatível com Win7 (testar `=1.14.0`; Win7 requer `GetSystemTimeAsFileTime` — versões ≥1.15 usam `GetSystemTimePreciseAsFileTime` que é Vista+).
- [ ] **T03** — Criar script `scripts/check_env.ps1` que valida presença de `mpv-1.dll` e variáveis de ambiente necessárias antes do build.

### 1.2 · Modelagem de Dados

- [ ] **T04** — Criar `src/types.rs` com structs `StreamNode` e enum `NodeStatus` conforme PRD §4.
  - Adicionar campo `last_checked: Option<chrono::DateTime<Utc>>` para health check.
  - Derivar `Clone`, `Debug`, `Serialize`, `Deserialize`, `Default`.
- [ ] **T05** — Criar `src/store.rs` responsável por carregar a lista de fontes:
  - Suporte a JSON (`sources.json`) e CSV (`sources.csv`) via `serde_json` e `csv`.
  - Tratar campos opcionais: `secondary_url` e `ip_address` podem ser vazios/nulos.
  - Retornar `Vec<StreamNode>` ou `anyhow::Error`.
- [ ] **T06** — Criar `data/sources.csv` e `data/sources.example.json` com 3-5 entradas de exemplo (com e sem URL secundária, com e sem porta no IP).

### 1.3 · Engine de Playback (libmpv)

- [ ] **T07** — Criar `src/player.rs` com wrapper ao redor do contexto MPV:
  - `MpvPlayer::new()` — inicializa `mpv::Mpv`, seta `hwdec=auto` (DXVA2 no Windows).
  - `MpvPlayer::play(url: &str)` — carrega URL e inicia reprodução.
  - `MpvPlayer::stop()` / `pause()` / `resume()` / `set_volume(vol: f64)`.
- [ ] **T08** — Criar `src/fallback.rs` com a lógica de fallback:
  - Assinar eventos do MPV (`EndFile`, reason `Error`).
  - Se `secondary_url` existir e ainda não foi tentada, invocar `player.play(secondary_url)` e atualizar `status = NodeStatus::Degraded`.
  - Expor `play_with_fallback(node: &mut StreamNode, player: &MpvPlayer)`.
- [ ] **T09** — Implementar CLI mínimo em `src/main.rs` (Fase 1):
  - Aceitar argumento `--endpoint <nome>` para localizar o `StreamNode` na lista.
  - Invocar `play_with_fallback` e manter o processo ativo até Ctrl+C.
  - Logar eventos relevantes via `log::info!` / `log::error!`.

### 1.4 · Validação da Fase 1

- [ ] **T10** — Teste manual: reproduzir stream real com URL primária OK.
- [ ] **T11** — Teste manual: simular URL primária inválida, confirmar fallback para secundária.
- [ ] **T12** — Confirmar binário release compilado (`cargo build --release --target x86_64-pc-windows-gnu`) e executado em VM Windows 7 SP1 64-bit.

---

## Fase 2 · Interface Gráfica (egui + render do MPV)

> Objetivo: janela eframe com sidebar de endpoints à esquerda e vídeo embutido à direita.

### 2.1 · Scaffold da UI

- [ ] **T13** — Criar `src/app.rs` com struct `App` implementando `eframe::App`:
  - Campos: `nodes: Vec<StreamNode>`, `selected: Option<usize>`, `player: MpvPlayer`.
  - `App::new(cc, nodes)` — inicializa estado.
- [ ] **T14** — Implementar sidebar (painel esquerdo):
  - Lista scrollável de `StreamNode` com nome do endpoint.
  - Indicador de status colorido: verde (Online), amarelo (Degraded), vermelho (Offline), cinza (Unknown).
  - Clique seleciona o node e dispara `play_with_fallback`.

### 2.2 · Render do Vídeo no egui

- [ ] **T15** — Pesquisar e definir a estratégia de render do MPV no egui:
  - **Opção A (recomendada):** MPV renderiza em janela própria embutida via `wid` (Window ID). Passar o HWND do painel central do egui para `mpv_set_option("wid", hwnd)`.
  - **Opção B:** MPV render context com OpenGL offscreen → textura egui. Mais complexo, melhor integração visual.
  - Documentar escolha em `docs/video_render_decision.md`.
- [ ] **T16** — Implementar integração de render escolhida (T15):
  - Se Opção A: obter HWND do painel via `raw_window_handle` e passar para MPV.
  - Se Opção B: criar `mpv_render_context`, extrair frame como textura OpenGL e registrar como `egui::TextureHandle`.
- [ ] **T17** — Painel de controles de mídia (abaixo do vídeo):
  - Botões Play/Pause, Mute, slider de Volume.
  - Exibir URL ativa (primária ou secundária) e `NodeStatus` corrente.

### 2.3 · Controles e Responsividade

- [ ] **T18** — Garantir que o loop MPV (`event_loop`) roda em thread separada da thread da UI (evitar congelamento em caso de buffer engasgado).
- [ ] **T19** — Comunicação UI ↔ Player via canais (`std::sync::mpsc` ou `tokio::sync::watch`) para:
  - UI envia comandos: `Play(url)`, `Pause`, `Resume`, `SetVolume(f64)`.
  - Player envia estado: `StatusUpdate(NodeStatus)`, `PlaybackError(String)`.

### 2.4 · Validação da Fase 2

- [ ] **T20** — Teste de integração: clicar em endpoint na sidebar reproduz vídeo no painel.
- [ ] **T21** — Teste de failover visual: status muda para "Degraded" (amarelo) ao cair para secundária.
- [ ] **T22** — Build e execução em Windows 7 VM confirma renderização sem crash.

---

## Fase 3 · Health Check & Concorrência

> Objetivo: varredura periódica de todas as URLs/IPs em background, atualizando status visual em tempo real.

### 3.1 · Serviço de Health Check

- [ ] **T23** — Criar `src/health.rs` com `HealthService`:
  - Recebe `Arc<Mutex<Vec<StreamNode>>>` compartilhado com a UI.
  - Método `run(interval_secs: u64)` — loop tokio que itera todos os nodes.
  - Para cada node: fazer HTTP HEAD na `primary_url` (timeout 5s) via `ureq`.
  - Se falhar: tentar `secondary_url`; se ambas falharem → `Offline`; se só secundária OK → `Degraded`.
- [ ] **T24** — Se `ip_address` presente: fazer TCP connect (porta 80 ou a porta do IP se especificada) como check complementar.
  - Parsear `ip:porta` vs apenas `ip` tratando porta default como 80.
- [ ] **T25** — Configurar intervalo via campo na UI ou arquivo de config (`config.json`):
  - Campo `health_check_interval_secs: u64` (default: 300 — 5 minutos).
  - Carregar em `src/store.rs` junto com os sources.

### 3.2 · Integração com a UI

- [ ] **T26** — Conectar `HealthService` ao estado compartilhado da `App`:
  - Lançar tokio runtime em thread separada ao iniciar o eframe.
  - UI relê `Arc<Mutex<Vec<StreamNode>>>` a cada frame para atualizar indicadores.
- [ ] **T27** — Adicionar `last_checked` timestamp na sidebar (ex: "2 min atrás") usando `chrono`.
- [ ] **T28** — Botão "Verificar Agora" na UI para disparar varredura imediata fora do ciclo agendado.

### 3.3 · Polimento e Resiliência

- [ ] **T29** — Garantir que panic em thread de health check não mata a UI: usar `std::panic::catch_unwind` ou `tokio::spawn` com `JoinHandle` monitorado.
- [ ] **T30** — Persistência de estado: salvar último status conhecido de cada node em `state.json` (via `serde_json`) para restaurar indicadores ao reiniciar sem esperar nova varredura.
- [ ] **T31** — Logging estruturado: registrar cada health check (URL, resultado, latência) em arquivo rotativo (`mx_player.log`) usando `env_logger` com filtro configurável.

### 3.4 · Validação da Fase 3

- [ ] **T32** — Teste: matar a URL primária de um node enquanto o app roda, confirmar que o indicador muda para Degraded/Offline após o próximo ciclo.
- [ ] **T33** — Teste de estabilidade: deixar rodando por 1 hora com varredura a cada 60s, confirmar sem leak de memória ou threads acumuladas.
- [ ] **T34** — Build release final e teste completo em Windows 7 SP1 64-bit.

---

## Tarefas Transversais (todas as fases)

- [ ] **TX01** — Definir e documentar convenção de nomenclatura dos arquivos de source (campo `endpoint_name`) para alinhar com a lista real de streams.
- [ ] **TX02** — Criar `.github/`-free CI local: `scripts/build_release.ps1` que compila para `x86_64-pc-windows-gnu` e `x86_64-unknown-linux-gnu`.
- [ ] **TX03** — Resolver bundling do `mpv-1.dll` no release Windows: copiar DLL junto ao `.exe` ou embutir via `include_bytes!` (avaliar tamanho).
- [ ] **TX04** — Avaliar e documentar consumo de CPU em idle (player parado) e durante reprodução ativa para garantir NFR01.

---

## Dependências Bloqueantes

| Task    | Depende de              | Risco                                                                            |
| ------- | ----------------------- | -------------------------------------------------------------------------------- |
| T07–T09 | T01 (libmpv no Win7)    | **Alto** — libmpv requer DLL externa; Win7 pode precisar de versão antiga do mpv |
| T16     | T15 (decisão de render) | Médio — Opção B é complexa mas mais integrada                                    |
| T23–T26 | T02 (tokio Win7)        | Médio — se tokio 1.14 não funcionar, migrar health check para threads nativas    |
| T34     | T22, T33                | Baixo — validação final                                                          |

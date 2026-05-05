# MX Player

## Overview
Player de vídeo multiplataforma focado em streams de rede, com interface gráfica (egui + libmpv), lógica de fallback automático entre URLs e health check periódico dos endpoints. Requisito obrigatório: o binário release deve rodar em **Windows 7 SP1 64-bit** sem dependências adicionais além do `mpv-1.dll`. 

## Instructions
> **Antes de iniciar qualquer tarefa, leia o arquivo `TASKS.md` na raiz do projeto.** Ele contém o detalhamento completo de cada subtarefa, dependências bloqueantes, alternativas técnicas e critérios de validação que devem guiar a implementação.

## Tasks
- [ ] Task 1: Configurar ambiente de build — integrar `libmpv` no Windows (binários MinGW compatíveis com Win7), validar versão do tokio compatível com Win7 (≤1.14.0) e criar script `scripts/check_env.ps1` de verificação de dependências
- [ ] Task 2: Modelar dados e camada de store — criar `src/types.rs` (structs `StreamNode`, enum `NodeStatus`), `src/store.rs` com suporte a JSON e CSV, e arquivos de exemplo em `data/`
- [ ] Task 3: Implementar engine de playback e fallback — criar `src/player.rs` (wrapper libmpv com hwdec DXVA2), `src/fallback.rs` (lógica de troca para URL secundária) e CLI mínimo em `src/main.rs` com argumento `--endpoint`
- [ ] Task 4: Construir interface gráfica egui — criar `src/app.rs` com sidebar de endpoints (indicadores de status coloridos), painel de vídeo com integração de render MPV via HWND e controles de mídia (Play/Pause, Volume, URL ativa)
- [ ] Task 5: Implementar health check concorrente — criar `src/health.rs` com `HealthService` usando HTTP HEAD + TCP connect, estado compartilhado via `Arc<Mutex<>>` com a UI, persistência em `state.json` e logging rotativo
- [ ] Task 6: Validação e release final — testes de fallback e estabilidade, bundling do `mpv-1.dll` junto ao `.exe`, script `scripts/build_release.ps1` para target `x86_64-pc-windows-gnu` e execução confirmada em VM Windows 7 SP1 64-bit

## Technical Details
- **Linguagem:** Rust (edition 2021)
- **Target obrigatório:** `x86_64-pc-windows-gnu` (Windows 7 SP1+)
- **Player:** `libmpv` — binários MinGW; verificar compatibilidade com Win7 (evitar versões que dependam de `ucrt`); hwdec via DXVA2
- **UI:** `eframe`/`egui` — render do vídeo via HWND (`wid` option do MPV) como estratégia principal
- **Async:** `tokio ≤1.14.0` (Win7 não suporta `GetSystemTimePreciseAsFileTime`, presente no tokio ≥1.15); alternativa de fallback: threads nativas para health check
- **HTTP:** `ureq` (sem async, evita complexidade tokio em health check)
- **Serialização:** `serde_json`, `csv`
- **Logging:** `env_logger` com arquivo rotativo
- **Risco alto:** integração libmpv Win7 (T01) bloqueia toda a Fase 1; deve ser validada primeiro

//! Interface gráfica egui — janela principal do MX Player.
//!
//! Estrutura:
//! - `App` implementa `eframe::App`
//! - Sidebar esquerda: lista scrollável de endpoints com indicadores de status coloridos
//! - Painel central: área de vídeo (HWND filho Win32 para embedding MPV) + controles
//! - Thread de player em background comunica via `std::sync::mpsc`

use std::sync::{Arc, Mutex};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;

use chrono::Utc;

use egui::{Color32, RichText, ScrollArea, Slider, Vec2};

use crate::types::{NodeStatus, StreamNode};

// ============================================================================
// Tipos de mensagem (UI ↔ thread do player)
// ============================================================================

/// Comandos enviados da UI para a thread do player.
pub enum PlayerCommand {
    /// Inicia (ou substitui) reprodução — inclui URL primária e secundária (fallback).
    Play {
        primary_url: String,
        secondary_url: Option<String>,
        node_idx: usize,
    },
    /// Pausa a reprodução em andamento.
    Pause,
    /// Retoma a reprodução pausada.
    Resume,
    /// Para completamente a reprodução.
    Stop,
    /// Ajusta o volume (0.0–100.0).
    SetVolume(f64),
    /// Informa o HWND da janela filho de vídeo ao player (embedding MPV).
    SetWindow(i64),
    /// Solicita encerramento da thread do player.
    Quit,
}

/// Eventos enviados da thread do player para a UI.
pub enum PlayerEvent {
    /// Status de um node foi atualizado pelo engine de fallback.
    StatusUpdate { node_idx: usize, status: NodeStatus },
    /// URL atualmente em reprodução (None = parado).
    ActiveUrl(Option<String>),
    /// Erro de reprodução (para exibição na UI).
    PlaybackError(String),
    /// Player entrou em pausa.
    Paused,
    /// Player retomou reprodução.
    Resumed,
}

// ============================================================================
// Estado da reprodução na thread do player
// ============================================================================

/// Estado interno de reprodução mantido pela thread do player.
struct PlaybackState {
    node_idx: usize,
    /// URLs ordenadas: [primária, secundária (se existir)]
    urls: Vec<String>,
    /// Índice da URL sendo usada (0 = primária, 1 = secundária).
    attempt: usize,
}

// ============================================================================
// App principal
// ============================================================================

/// Estado principal da aplicação egui.
pub struct App {
    /// Lista de endpoints carregada do disco.
    nodes: Vec<StreamNode>,
    /// Índice do endpoint selecionado na sidebar.
    selected: Option<usize>,
    /// Volume atual (0.0–100.0).
    volume: f64,
    /// Se a reprodução está pausada.
    is_paused: bool,
    /// URL atualmente em reprodução.
    active_url: Option<String>,
    /// Canal de comandos para a thread do player.
    cmd_tx: Sender<PlayerCommand>,
    /// Canal de eventos vindo da thread do player.
    event_rx: Receiver<PlayerEvent>,
    /// HWND da janela principal eframe (obtido na primeira renderização, Windows).
    #[cfg(windows)]
    main_hwnd: Option<isize>,
    /// Janela filho Win32 usada como superfície de renderização do MPV.
    #[cfg(windows)]
    video_child: Option<VideoChildWindow>,
    /// Último rect do painel de vídeo (para reposicionar a janela filho ao redimensionar).
    last_video_rect: Option<egui::Rect>,
    /// Se o HWND de vídeo já foi enviado ao player.
    hwnd_configured: bool,
    /// Mensagem de erro a exibir na UI (limpa ao iniciar nova reprodução).
    error_msg: Option<String>,
    /// Estado compartilhado com o `HealthService` — sincronizado a cada frame.
    shared_nodes: Arc<Mutex<Vec<StreamNode>>>,
    /// Canal de disparo para varredura manual do health check ("Verificar Agora").
    health_trigger: Option<std::sync::mpsc::SyncSender<()>>,
}

impl App {
    /// Cria a aplicação, inicializa canais e spawna a thread do player.
    pub fn new(
        _cc: &eframe::CreationContext<'_>,
        nodes: Vec<StreamNode>,
        shared_nodes: Arc<Mutex<Vec<StreamNode>>>,
        health_trigger: Option<std::sync::mpsc::SyncSender<()>>,
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<PlayerCommand>();
        let (event_tx, event_rx) = mpsc::channel::<PlayerEvent>();

        // Spawn da thread do player em background — não bloqueia a UI
        {
            let event_tx = event_tx;
            std::thread::spawn(move || {
                run_player_thread(cmd_rx, event_tx);
            });
        }

        Self {
            nodes,
            selected: None,
            volume: 80.0,
            is_paused: false,
            active_url: None,
            cmd_tx,
            event_rx,
            #[cfg(windows)]
            main_hwnd: None,
            #[cfg(windows)]
            video_child: None,
            last_video_rect: None,
            hwnd_configured: false,
            error_msg: None,
            shared_nodes,
            health_trigger,
        }
    }

    /// Sincroniza `status` e `last_checked` dos nodes a partir do estado compartilhado
    /// com o `HealthService`. Usa `try_lock` para nunca bloquear o frame de renderização.
    fn sync_from_health(&mut self) {
        if let Ok(shared) = self.shared_nodes.try_lock() {
            for (i, shared_node) in shared.iter().enumerate() {
                if let Some(node) = self.nodes.get_mut(i) {
                    // Aplica apenas quando o health check já realizou ao menos uma checagem.
                    if shared_node.last_checked.is_some() {
                        node.status = shared_node.status.clone();
                        node.last_checked = shared_node.last_checked;
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Lógica de controle
    // -----------------------------------------------------------------------

    /// Seleciona um endpoint e inicia reprodução com fallback automático.
    fn select_and_play(&mut self, idx: usize) {
        if let Some(node) = self.nodes.get(idx) {
            let primary_url = node.primary_url.clone();
            let secondary_url = node.secondary_url.clone();
            self.selected = Some(idx);
            self.is_paused = false;
            self.error_msg = None;
            let _ = self.cmd_tx.send(PlayerCommand::Play {
                primary_url,
                secondary_url,
                node_idx: idx,
            });
        }
    }

    /// Processa eventos recebidos da thread do player (sem bloquear).
    fn process_player_events(&mut self) {
        loop {
            match self.event_rx.try_recv() {
                Ok(PlayerEvent::StatusUpdate { node_idx, status }) => {
                    if let Some(node) = self.nodes.get_mut(node_idx) {
                        node.status = status;
                    }
                }
                Ok(PlayerEvent::ActiveUrl(url)) => {
                    self.active_url = url;
                }
                Ok(PlayerEvent::PlaybackError(msg)) => {
                    log::error!("[app] Erro de reprodução: {}", msg);
                    self.error_msg = Some(msg);
                }
                Ok(PlayerEvent::Paused) => {
                    self.is_paused = true;
                }
                Ok(PlayerEvent::Resumed) => {
                    self.is_paused = false;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    // -----------------------------------------------------------------------
    // Renderização da sidebar
    // -----------------------------------------------------------------------

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Endpoints");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(ref trigger) = self.health_trigger {
                    if ui
                        .small_button("🔄")
                        .on_hover_text("Verificar Agora")
                        .clicked()
                    {
                        let _ = trigger.try_send(());
                        log::info!("[app] Varredura manual de health check disparada.");
                    }
                }
            });
        });
        ui.separator();

        // Coletar índices para evitar borrow duplo
        let count = self.nodes.len();

        if count == 0 {
            ui.label(
                RichText::new("Nenhum endpoint encontrado.\nCrie data/sources.csv")
                    .color(Color32::GRAY)
                    .small(),
            );
            return;
        }

        let selected = self.selected;
        ScrollArea::vertical()
            .id_source("sidebar_scroll")
            .show(ui, |ui| {
                for idx in 0..count {
                    let (name, status) = {
                        let node = &self.nodes[idx];
                        (node.name.clone(), node.status.clone())
                    };
                    let is_selected = selected == Some(idx);
                    let dot_color = status_color(&status);

                    let resp = ui.horizontal(|ui| {
                        // Indicador de status (círculo colorido)
                        let (dot_rect, _) =
                            ui.allocate_exact_size(Vec2::splat(14.0), egui::Sense::hover());
                        ui.painter()
                            .circle_filled(dot_rect.center(), 5.0, dot_color);
                        ui.painter().circle_stroke(
                            dot_rect.center(),
                            5.0,
                            egui::Stroke::new(1.0, Color32::from_gray(80)),
                        );

                        // Botão com nome do endpoint
                        let btn = egui::Button::new(
                            RichText::new(&name).color(if is_selected {
                                Color32::WHITE
                            } else {
                                Color32::LIGHT_GRAY
                            }),
                        )
                        .fill(if is_selected {
                            Color32::from_rgb(50, 80, 140)
                        } else {
                            Color32::TRANSPARENT
                        })
                        .min_size(Vec2::new(ui.available_width(), 22.0));

                        ui.add(btn)
                    });

                    if resp.inner.clicked() {
                        self.select_and_play(idx);
                    }

                    // Tooltip com detalhes do endpoint
                    resp.inner.on_hover_ui(|ui| {
                        let node = &self.nodes[idx];
                        ui.label(format!("URL: {}", node.primary_url));
                        if let Some(ref sec) = node.secondary_url {
                            ui.label(format!("Fallback: {}", sec));
                        }
                        if let Some(ref ip) = node.ip_address {
                            ui.label(format!("IP: {}", ip));
                        }
                        ui.label(format!("Status: {:?}", node.status));
                        if let Some(ts) = node.last_checked {
                            let ago_mins = Utc::now()
                                .signed_duration_since(ts)
                                .num_minutes();
                            let label = if ago_mins < 1 {
                                "agora há pouco".to_string()
                            } else {
                                format!("{} min atrás", ago_mins)
                            };
                            ui.label(
                                RichText::new(format!("Verificado: {}", label))
                                    .color(Color32::from_gray(180))
                                    .small(),
                            );
                        }
                    });
                }
            });
    }

    // -----------------------------------------------------------------------
    // Renderização dos controles de mídia
    // -----------------------------------------------------------------------

    fn render_controls(&mut self, ui: &mut egui::Ui) {
        ui.separator();

        let can_control = self.selected.is_some() && self.active_url.is_some();

        ui.horizontal(|ui| {
            // Play / Pause
            let pp_label = if self.is_paused {
                "▶  Retomar"
            } else {
                "⏸  Pausar"
            };
            if ui
                .add_enabled(can_control, egui::Button::new(pp_label))
                .clicked()
            {
                if self.is_paused {
                    let _ = self.cmd_tx.send(PlayerCommand::Resume);
                } else {
                    let _ = self.cmd_tx.send(PlayerCommand::Pause);
                }
            }

            // Stop
            if ui
                .add_enabled(can_control, egui::Button::new("⏹  Parar"))
                .clicked()
            {
                let _ = self.cmd_tx.send(PlayerCommand::Stop);
                self.active_url = None;
                self.is_paused = false;
            }

            ui.separator();

            // Volume
            ui.label("🔊");
            let old_vol = self.volume;
            ui.add(
                Slider::new(&mut self.volume, 0.0_f64..=100.0_f64)
                    .step_by(1.0)
                    .suffix("%")
                    .max_decimals(0),
            );
            if (self.volume - old_vol).abs() > f64::EPSILON {
                let _ = self.cmd_tx.send(PlayerCommand::SetVolume(self.volume));
            }
        });

        ui.horizontal(|ui| {
            ui.label("URL ativa:");
            let url_text = self.active_url.as_deref().unwrap_or("— nenhuma —");
            ui.label(
                RichText::new(url_text)
                    .color(Color32::from_rgb(100, 180, 255))
                    .monospace()
                    .small(),
            );
        });

        // Status do node selecionado
        if let Some(idx) = self.selected {
            if let Some(node) = self.nodes.get(idx) {
                ui.horizontal(|ui| {
                    ui.label("Status:");
                    let color = status_color(&node.status);
                    ui.label(RichText::new(format!("{:?}", node.status)).color(color));
                });
            }
        }

        // Mensagem de erro (se houver)
        if let Some(ref msg) = self.error_msg {
            ui.label(
                RichText::new(format!("⚠ {}", msg))
                    .color(Color32::from_rgb(255, 120, 80))
                    .small(),
            );
        }
    }
}

// ============================================================================
// eframe::App
// ============================================================================

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // 1. Processar eventos da thread do player
        self.process_player_events();

        // 1b. Sincronizar status/last_checked com o HealthService
        self.sync_from_health();

        // 2. Obter HWND da janela principal na primeira renderização (Windows)
        #[cfg(windows)]
        {
            if self.main_hwnd.is_none() {
                self.main_hwnd = get_main_hwnd(frame);
            }
        }

        // 3. Sidebar esquerda
        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(230.0)
            .min_width(160.0)
            .show(ctx, |ui| {
                self.render_sidebar(ui);
            });

        // 4. Painel central: vídeo + controles
        egui::CentralPanel::default().show(ctx, |ui| {
            let available = ui.available_size();
            let controls_height = 90.0_f32;
            let video_height = (available.y - controls_height).max(1.0);

            // Área de vídeo — fundo preto
            let (video_rect, _) = ui.allocate_exact_size(
                Vec2::new(available.x, video_height),
                egui::Sense::hover(),
            );
            ui.painter().rect_filled(video_rect, 0.0, Color32::BLACK);

            // Overlay de texto quando nenhum stream está ativo
            if self.active_url.is_none() {
                ui.painter().text(
                    video_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Selecione um endpoint na barra lateral",
                    egui::FontId::proportional(15.0),
                    Color32::from_gray(100),
                );
            }

            // 5. Configurar janela filho para MPV (Windows)
            #[cfg(windows)]
            self.setup_video_hwnd(ctx, video_rect);

            // 6. Controles de mídia
            self.render_controls(ui);
        });

        // Solicitar repaint periódico para atualizar status dos nodes
        ctx.request_repaint_after(Duration::from_millis(200));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.cmd_tx.send(PlayerCommand::Quit);
    }
}

// ============================================================================
// Integração HWND — Windows
// ============================================================================

/// Obtém o HWND da janela eframe principal via `raw-window-handle`.
#[cfg(windows)]
fn get_main_hwnd(frame: &eframe::Frame) -> Option<isize> {
    use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};
    let rwh = frame.raw_window_handle();
    if let RawWindowHandle::Win32(h) = rwh {
        Some(h.hwnd as isize)
    } else {
        None
    }
}

/// Janela filho Win32 dedicada à renderização do MPV.
///
/// O MPV recebe o HWND desta janela como `wid`, renderizando o vídeo dentro
/// dela. A janela é posicionada/redimensionada conforme o painel de vídeo do egui.
#[cfg(windows)]
pub struct VideoChildWindow {
    hwnd: winapi::shared::windef::HWND,
}

#[cfg(windows)]
impl VideoChildWindow {
    /// Cria a janela filho como child de `parent`, posicionada em `(x, y)` com
    /// dimensões `w × h` (em pixels de tela).
    pub fn create(
        parent: winapi::shared::windef::HWND,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> Option<Self> {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use winapi::um::libloaderapi::GetModuleHandleW;
        use winapi::um::winuser::*;

        // Nome de classe único para a janela filho MPV
        let class_name: Vec<u16> = OsStr::new("MXPlayerVideoWnd\0")
            .encode_wide()
            .collect();
        let window_title: Vec<u16> = OsStr::new("MPV Video\0").encode_wide().collect();

        unsafe {
            let hinstance = GetModuleHandleW(std::ptr::null());

            let mut wc: WNDCLASSEXW = std::mem::zeroed();
            wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
            wc.style = CS_HREDRAW | CS_VREDRAW;
            wc.lpfnWndProc = Some(DefWindowProcW);
            wc.hInstance = hinstance;
            // Fundo preto — MPV vai sobrescrever
            wc.hbrBackground = winapi::um::winuser::COLOR_WINDOWFRAME
                as winapi::shared::windef::HBRUSH;
            wc.lpszClassName = class_name.as_ptr();

            // Ignorar erro "classe já registrada" (ocorre em múltiplas janelas)
            RegisterClassExW(&wc);

            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                window_title.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
                x,
                y,
                w.max(1),
                h.max(1),
                parent,
                std::ptr::null_mut(),
                hinstance,
                std::ptr::null_mut(),
            );

            if hwnd.is_null() {
                log::error!("[app] Falha ao criar janela filho para MPV (CreateWindowExW)");
                None
            } else {
                log::info!("[app] Janela filho MPV criada: HWND={:p}", hwnd);
                Some(Self { hwnd })
            }
        }
    }

    /// Reposiciona e redimensiona a janela filho.
    pub fn set_position(&self, x: i32, y: i32, w: i32, h: i32) {
        use winapi::um::winuser::{SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER};
        unsafe {
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                x,
                y,
                w.max(1),
                h.max(1),
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    /// Retorna o HWND como `i64` para passar ao MPV via `set_window`.
    pub fn hwnd_i64(&self) -> i64 {
        self.hwnd as i64
    }
}

#[cfg(windows)]
impl Drop for VideoChildWindow {
    fn drop(&mut self) {
        unsafe {
            winapi::um::winuser::DestroyWindow(self.hwnd);
        }
    }
}

// VideoChildWindow contém *mut c_void (HWND), o que já torna o tipo !Send + !Sync
// automaticamente em Rust — não é necessário impl !Send explícito.

#[cfg(windows)]
impl App {
    /// Cria ou reposiciona a janela filho de vídeo MPV com base no rect do painel egui.
    fn setup_video_hwnd(&mut self, ctx: &egui::Context, video_rect: egui::Rect) {
        let main_hwnd = match self.main_hwnd {
            Some(h) => h as winapi::shared::windef::HWND,
            None => return,
        };

        // Converter coordenadas egui (pontos lógicos) para pixels de tela
        let ppp = ctx.pixels_per_point();
        let x = (video_rect.min.x * ppp) as i32;
        let y = (video_rect.min.y * ppp) as i32;
        let w = (video_rect.width() * ppp) as i32;
        let h = (video_rect.height() * ppp) as i32;

        match self.video_child {
            Some(ref child) => {
                // Reposicionar se o rect mudou
                if self.last_video_rect != Some(video_rect) {
                    child.set_position(x, y, w, h);
                    self.last_video_rect = Some(video_rect);
                }
            }
            None => {
                // Criar janela filho pela primeira vez
                if let Some(child) = VideoChildWindow::create(main_hwnd, x, y, w, h) {
                    if !self.hwnd_configured {
                        let hwnd_val = child.hwnd_i64();
                        log::info!("[app] Enviando wid={} ao player", hwnd_val);
                        let _ = self.cmd_tx.send(PlayerCommand::SetWindow(hwnd_val));
                        self.hwnd_configured = true;
                    }
                    self.video_child = Some(child);
                    self.last_video_rect = Some(video_rect);
                }
            }
        }
    }
}

// ============================================================================
// Thread do player em background
// ============================================================================

/// Roda na thread do player — inicializa MPV, recebe comandos da UI e emite eventos.
///
/// O loop de eventos MPV (`ev_ctx.wait_event`) usa timeout curto (50 ms) para
/// que os comandos da UI sejam processados com baixa latência.
///
/// Compilado apenas quando **não** estamos em modo teste, pois MPV requer
/// `mpv-1.dll` que pode não estar presente em CI.
#[cfg(not(test))]
fn run_player_thread(cmd_rx: Receiver<PlayerCommand>, event_tx: Sender<PlayerEvent>) {
    use crate::fallback::{next_url, update_status};
    use crate::player::MpvPlayer;
    use crate::types::StreamNode;
    use libmpv::events::Event;

    let player = match MpvPlayer::new() {
        Ok(p) => p,
        Err(e) => {
            log::error!("[player_thread] Falha ao inicializar MPV: {:#}", e);
            let _ = event_tx.send(PlayerEvent::PlaybackError(format!(
                "Falha ao inicializar MPV: {}",
                e
            )));
            return;
        }
    };

    let mut ev_ctx = player.create_event_context();

    // Estado de reprodução atual (None = parado)
    let mut state: Option<PlaybackState> = None;

    log::info!("[player_thread] Iniciado.");

    loop {
        // -----------------------------------------------------------------
        // Processar evento MPV (50 ms de espera máxima)
        // -----------------------------------------------------------------
        match ev_ctx.wait_event(0.05) {
            Some(Err(e)) => {
                // Erro de stream → tentar fallback para URL secundária
                log::warn!("[player_thread] Erro MPV: {}", e);
                if let Some(ref mut s) = state {
                    s.attempt += 1;
                    // Montar um StreamNode temporário para reutilizar next_url/update_status
                    let mut dummy = StreamNode::default();
                    dummy.primary_url = s.urls.first().cloned().unwrap_or_default();
                    dummy.secondary_url = s.urls.get(1).cloned();

                    match next_url(&dummy, s.attempt) {
                        Some(url) => {
                            let url = url.to_owned();
                            log::info!(
                                "[player_thread] Fallback para URL secundária: {}",
                                url
                            );
                            if let Err(pe) = player.play(&url) {
                                log::error!("[player_thread] play fallback: {}", pe);
                                let _ = event_tx
                                    .send(PlayerEvent::PlaybackError(pe.to_string()));
                            } else {
                                update_status(&mut dummy, s.attempt);
                                let _ = event_tx.send(PlayerEvent::StatusUpdate {
                                    node_idx: s.node_idx,
                                    status: dummy.status.clone(),
                                });
                                let _ = event_tx
                                    .send(PlayerEvent::ActiveUrl(Some(url)));
                            }
                        }
                        None => {
                            // Todas as URLs falharam
                            update_status(&mut dummy, s.attempt);
                            let _ = event_tx.send(PlayerEvent::StatusUpdate {
                                node_idx: s.node_idx,
                                status: dummy.status.clone(),
                            });
                            let _ = event_tx.send(PlayerEvent::PlaybackError(
                                "Todas as URLs falharam".to_string(),
                            ));
                            let _ = event_tx.send(PlayerEvent::ActiveUrl(None));
                            state = None;
                        }
                    }
                }
            }
            Some(Ok(Event::Shutdown)) => {
                log::info!("[player_thread] Shutdown MPV recebido.");
                break;
            }
            _ => {}
        }

        // -----------------------------------------------------------------
        // Processar comandos da UI (não-bloqueante)
        // -----------------------------------------------------------------
        match cmd_rx.try_recv() {
            Ok(PlayerCommand::Quit) => {
                log::info!("[player_thread] Quit recebido.");
                let _ = player.stop();
                break;
            }
            Ok(PlayerCommand::Play {
                primary_url,
                secondary_url,
                node_idx,
            }) => {
                log::info!("[player_thread] Play: {}", primary_url);
                let mut urls = vec![primary_url.clone()];
                if let Some(sec) = secondary_url {
                    urls.push(sec);
                }
                state = Some(PlaybackState {
                    node_idx,
                    urls,
                    attempt: 0,
                });
                if let Err(e) = player.play(&primary_url) {
                    log::error!("[player_thread] play: {}", e);
                    let _ = event_tx.send(PlayerEvent::PlaybackError(e.to_string()));
                } else {
                    let _ = event_tx.send(PlayerEvent::StatusUpdate {
                        node_idx,
                        status: NodeStatus::Online,
                    });
                    let _ = event_tx.send(PlayerEvent::ActiveUrl(Some(primary_url)));
                }
            }
            Ok(PlayerCommand::Pause) => {
                if let Err(e) = player.pause() {
                    log::error!("[player_thread] pause: {}", e);
                } else {
                    let _ = event_tx.send(PlayerEvent::Paused);
                }
            }
            Ok(PlayerCommand::Resume) => {
                if let Err(e) = player.resume() {
                    log::error!("[player_thread] resume: {}", e);
                } else {
                    let _ = event_tx.send(PlayerEvent::Resumed);
                }
            }
            Ok(PlayerCommand::Stop) => {
                if let Err(e) = player.stop() {
                    log::error!("[player_thread] stop: {}", e);
                }
                if let Some(ref s) = state {
                    let _ = event_tx.send(PlayerEvent::StatusUpdate {
                        node_idx: s.node_idx,
                        status: NodeStatus::Unknown,
                    });
                }
                state = None;
                let _ = event_tx.send(PlayerEvent::ActiveUrl(None));
            }
            Ok(PlayerCommand::SetVolume(vol)) => {
                if let Err(e) = player.set_volume(vol) {
                    log::error!("[player_thread] set_volume: {}", e);
                }
            }
            Ok(PlayerCommand::SetWindow(hwnd)) => {
                log::info!("[player_thread] set_window wid={}", hwnd);
                if let Err(e) = player.set_window(hwnd) {
                    log::error!("[player_thread] set_window: {}", e);
                }
            }
            Err(TryRecvError::Empty) => {
                // Nenhum comando pendente — continuar o loop
            }
            Err(TryRecvError::Disconnected) => {
                log::info!("[player_thread] Canal de comandos desconectado — encerrando.");
                break;
            }
        }
    }

    log::info!("[player_thread] Encerrado.");
}

/// Versão stub da thread do player para testes (sem mpv-1.dll).
#[cfg(test)]
fn run_player_thread(cmd_rx: Receiver<PlayerCommand>, event_tx: Sender<PlayerEvent>) {
    loop {
        match cmd_rx.recv() {
            Ok(PlayerCommand::Quit) | Err(_) => break,
            Ok(PlayerCommand::Play {
                primary_url,
                node_idx,
                ..
            }) => {
                let _ = event_tx.send(PlayerEvent::StatusUpdate {
                    node_idx,
                    status: NodeStatus::Online,
                });
                let _ = event_tx.send(PlayerEvent::ActiveUrl(Some(primary_url)));
            }
            Ok(PlayerCommand::Stop) => {
                let _ = event_tx.send(PlayerEvent::ActiveUrl(None));
            }
            Ok(PlayerCommand::Pause) => {
                let _ = event_tx.send(PlayerEvent::Paused);
            }
            Ok(PlayerCommand::Resume) => {
                let _ = event_tx.send(PlayerEvent::Resumed);
            }
            Ok(PlayerCommand::SetVolume(_))
            | Ok(PlayerCommand::SetWindow(_)) => {}
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Retorna a cor associada a um `NodeStatus` para uso na UI.
pub fn status_color(status: &NodeStatus) -> Color32 {
    match status {
        NodeStatus::Online => Color32::from_rgb(80, 200, 80),    // verde
        NodeStatus::Degraded => Color32::from_rgb(255, 200, 0),  // amarelo
        NodeStatus::Offline => Color32::from_rgb(220, 60, 60),   // vermelho
        NodeStatus::Unknown => Color32::from_rgb(150, 150, 150), // cinza
    }
}

// ============================================================================
// Testes
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // ------------------------------------------------------------------
    // status_color
    // ------------------------------------------------------------------

    #[test]
    fn status_color_online_is_greenish() {
        let c = status_color(&NodeStatus::Online);
        // Verde deve ter canal G alto e R/B menores
        assert!(c.g() > 150, "verde esperado no canal G");
        assert!(c.r() < c.g(), "online: G > R");
    }

    #[test]
    fn status_color_degraded_is_yellowish() {
        let c = status_color(&NodeStatus::Degraded);
        assert!(c.r() > 200, "amarelo: R alto");
        assert!(c.g() > 150, "amarelo: G alto");
        assert!(c.b() < 50, "amarelo: B baixo");
    }

    #[test]
    fn status_color_offline_is_reddish() {
        let c = status_color(&NodeStatus::Offline);
        assert!(c.r() > 150, "vermelho: R alto");
        assert!(c.g() < c.r(), "offline: R > G");
    }

    #[test]
    fn status_color_unknown_is_grayish() {
        let c = status_color(&NodeStatus::Unknown);
        // Cinza: R, G, B próximos
        let diff_rg = (c.r() as i32 - c.g() as i32).abs();
        let diff_gb = (c.g() as i32 - c.b() as i32).abs();
        assert!(diff_rg < 20, "cinza: R≈G");
        assert!(diff_gb < 20, "cinza: G≈B");
    }

    // ------------------------------------------------------------------
    // PlayerCommand / PlayerEvent — via thread stub
    // ------------------------------------------------------------------

    fn make_channel() -> (Sender<PlayerCommand>, Receiver<PlayerEvent>) {
        let (cmd_tx, cmd_rx) = mpsc::channel::<PlayerCommand>();
        let (evt_tx, evt_rx) = mpsc::channel::<PlayerEvent>();
        std::thread::spawn(move || run_player_thread(cmd_rx, evt_tx));
        (cmd_tx, evt_rx)
    }

    fn recv_all(rx: &Receiver<PlayerEvent>, millis: u64) -> Vec<String> {
        std::thread::sleep(Duration::from_millis(millis));
        let mut tags = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            match ev {
                PlayerEvent::ActiveUrl(Some(u)) => tags.push(format!("url:{}", u)),
                PlayerEvent::ActiveUrl(None) => tags.push("url:none".into()),
                PlayerEvent::StatusUpdate { status, .. } => {
                    tags.push(format!("status:{:?}", status))
                }
                PlayerEvent::Paused => tags.push("paused".into()),
                PlayerEvent::Resumed => tags.push("resumed".into()),
                PlayerEvent::PlaybackError(e) => tags.push(format!("error:{}", e)),
            }
        }
        tags
    }

    #[test]
    fn play_command_emits_status_and_url() {
        let (tx, rx) = make_channel();
        tx.send(PlayerCommand::Play {
            primary_url: "http://example.com/s.m3u8".into(),
            secondary_url: None,
            node_idx: 0,
        })
        .unwrap();
        let events = recv_all(&rx, 80);
        assert!(
            events.iter().any(|e| e.starts_with("status:")),
            "deve emitir StatusUpdate"
        );
        assert!(
            events.iter().any(|e| e.starts_with("url:")),
            "deve emitir ActiveUrl"
        );
    }

    #[test]
    fn stop_command_clears_url() {
        let (tx, rx) = make_channel();
        tx.send(PlayerCommand::Play {
            primary_url: "http://example.com/s.m3u8".into(),
            secondary_url: None,
            node_idx: 0,
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(30));
        tx.send(PlayerCommand::Stop).unwrap();
        let events = recv_all(&rx, 80);
        assert!(
            events.contains(&"url:none".to_string()),
            "Stop deve emitir ActiveUrl(None)"
        );
    }

    #[test]
    fn pause_emits_paused_event() {
        let (tx, rx) = make_channel();
        tx.send(PlayerCommand::Pause).unwrap();
        let events = recv_all(&rx, 80);
        assert!(events.contains(&"paused".to_string()));
    }

    #[test]
    fn resume_emits_resumed_event() {
        let (tx, rx) = make_channel();
        tx.send(PlayerCommand::Resume).unwrap();
        let events = recv_all(&rx, 80);
        assert!(events.contains(&"resumed".to_string()));
    }

    #[test]
    fn quit_terminates_thread() {
        let (tx, _rx) = make_channel();
        // Quit deve encerrar a thread sem pânico
        tx.send(PlayerCommand::Quit).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        // Se a thread encerrou, a próxima tentativa de envio falha
        let result = tx.send(PlayerCommand::Stop);
        assert!(result.is_err(), "Após Quit, o canal deve estar fechado");
    }

    #[test]
    fn set_volume_and_set_window_dont_crash() {
        let (tx, _rx) = make_channel();
        tx.send(PlayerCommand::SetVolume(75.0)).unwrap();
        tx.send(PlayerCommand::SetWindow(12345)).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        // Sem pânico = sucesso
    }
}

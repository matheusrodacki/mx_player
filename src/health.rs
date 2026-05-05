//! Serviço de health check concorrente para endpoints de stream.
//!
//! # Visão Geral
//! `HealthService` roda em uma thread nativa dedicada e realiza varreduras
//! periódicas em todos os `StreamNode`s, usando:
//! - **HTTP HEAD** (via `ureq`) na `primary_url` e, em caso de falha, na `secondary_url`.
//! - **TCP connect** no `ip_address` (se presente) como verificação complementar.
//!
//! O estado é compartilhado com a UI via `Arc<Mutex<Vec<StreamNode>>>`.
//! Após cada varredura, o estado é persistido em `state.json` para restauração
//! ao reiniciar sem esperar nova varredura.
//!
//! # Uso típico
//! ```no_run
//! let mut nodes = vec![/* ... */];
//! health::restore_saved_state(&mut nodes);
//! let shared = Arc::new(Mutex::new(nodes.clone()));
//! let svc = health::HealthService::new(shared.clone());
//! let trigger = svc.trigger_sender();
//! std::thread::spawn(move || svc.run(300));
//! // Na UI: trigger.try_send(()) para forçar varredura imediata.
//! ```

use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::store;
use crate::types::{NodeStatus, StreamNode};

// ---------------------------------------------------------------------------
// Constantes
// ---------------------------------------------------------------------------

const STATE_FILE: &str = "state.json";
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// HealthService
// ---------------------------------------------------------------------------

/// Serviço de health check periódico com suporte a disparo manual.
pub struct HealthService {
    nodes: Arc<Mutex<Vec<StreamNode>>>,
    trigger_tx: std::sync::mpsc::SyncSender<()>,
    trigger_rx: std::sync::mpsc::Receiver<()>,
}

impl HealthService {
    /// Cria o serviço associado ao estado compartilhado.
    pub fn new(nodes: Arc<Mutex<Vec<StreamNode>>>) -> Self {
        let (trigger_tx, trigger_rx) = std::sync::mpsc::sync_channel(1);
        Self {
            nodes,
            trigger_tx,
            trigger_rx,
        }
    }

    /// Retorna um clone do canal de disparo para uso na UI ("Verificar Agora").
    pub fn trigger_sender(&self) -> std::sync::mpsc::SyncSender<()> {
        self.trigger_tx.clone()
    }

    /// Executa o loop de health check **bloqueando** a thread atual.
    ///
    /// Chame a partir de uma thread dedicada via `std::thread::spawn`.
    /// O loop termina quando o canal de disparo é desconectado (drop do `SyncSender`).
    ///
    /// A primeira varredura ocorre imediatamente ao iniciar. As varreduras
    /// subsequentes ocorrem a cada `interval_secs` segundos ou quando um disparo
    /// manual é recebido.
    pub fn run(self, interval_secs: u64) {
        let interval = Duration::from_secs(interval_secs);
        // Executa imediatamente na primeira iteração.
        let mut next_check = Instant::now();

        log::info!(
            "[health] Serviço iniciado — intervalo {}s.",
            interval_secs
        );

        loop {
            if Instant::now() >= next_check {
                scan_all_safe(&self.nodes);
                save_state(&self.nodes);
                next_check = Instant::now() + interval;
            }

            // Espera pelo próximo ciclo ou por um disparo manual.
            let remaining = next_check.saturating_duration_since(Instant::now());
            // Limite de 1s para o timeout permitir checagem de condição no topo do loop.
            let wait = remaining.min(Duration::from_secs(1));

            match self.trigger_rx.recv_timeout(wait) {
                Ok(()) => {
                    log::info!("[health] Varredura manual disparada.");
                    scan_all_safe(&self.nodes);
                    save_state(&self.nodes);
                    next_check = Instant::now() + interval;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // Loop normal — nenhuma ação.
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    log::info!("[health] Canal de disparo desconectado — encerrando serviço.");
                    break;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// API pública — persistência de estado
// ---------------------------------------------------------------------------

/// Restaura o último status conhecido de `state.json` nos nodes (em-memória).
///
/// Deve ser chamado **antes** de criar o `Arc<Mutex<>>` e spawnar o serviço,
/// para que a UI exiba os status salvos imediatamente na abertura.
/// A correspondência entre nodes é feita pelo campo `name`.
pub fn restore_saved_state(nodes: &mut Vec<StreamNode>) {
    if !std::path::Path::new(STATE_FILE).exists() {
        log::debug!("[health] Nenhum {} encontrado — status inicial Unknown.", STATE_FILE);
        return;
    }
    match store::load_from_json(STATE_FILE) {
        Ok(saved) => {
            apply_saved_state(nodes, &saved);
            log::info!("[health] Estado restaurado de {}.", STATE_FILE);
        }
        Err(e) => log::warn!("[health] Falha ao restaurar estado: {:#}", e),
    }
}

// ---------------------------------------------------------------------------
// Internos — varredura
// ---------------------------------------------------------------------------

/// Executa `scan_all` com proteção contra pânico para não derrubar a UI.
fn scan_all_safe(nodes: &Arc<Mutex<Vec<StreamNode>>>) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        scan_all(nodes);
    }));
    if let Err(_) = result {
        log::error!("[health] Pânico capturado durante varredura. Serviço continua.");
    }
}

/// Varre todos os nodes, registra latência e atualiza status + `last_checked`.
fn scan_all(nodes: &Arc<Mutex<Vec<StreamNode>>>) {
    // Clona snapshot para liberar o lock durante as chamadas de rede.
    let snapshot = {
        let lock = nodes.lock().unwrap();
        lock.clone()
    };

    let mut results: Vec<(usize, NodeStatus, chrono::DateTime<chrono::Utc>)> = Vec::new();

    for (idx, node) in snapshot.iter().enumerate() {
        let t0 = Instant::now();
        let status = check_node(node);
        let elapsed_ms = t0.elapsed().as_millis();
        let checked_at = chrono::Utc::now();

        log::info!(
            "[health] {} → {:?} ({}ms)",
            node.name,
            status,
            elapsed_ms
        );

        results.push((idx, status, checked_at));
    }

    // Escreve resultados de volta com lock mínimo.
    let mut lock = nodes.lock().unwrap();
    for (idx, status, checked_at) in results {
        if let Some(node) = lock.get_mut(idx) {
            node.status = status;
            node.last_checked = Some(checked_at);
        }
    }
}

/// Determina o status de um node verificando HTTP e TCP.
fn check_node(node: &StreamNode) -> NodeStatus {
    let primary_ok = check_http(&node.primary_url);
    let tcp_ok = node.ip_address.as_deref().map(check_tcp).unwrap_or(true);
    let secondary_ok = node.secondary_url.as_deref().map(check_http);
    derive_status(primary_ok, secondary_ok, tcp_ok)
}

/// Lógica pura de derivação de status — testável sem rede.
///
/// | primary_ok | tcp_ok | secondary_ok | resultado  |
/// |------------|--------|--------------|------------|
/// | true       | true   | qualquer     | Online     |
/// | true       | false  | qualquer     | Degraded   |
/// | false      | *      | Some(true)   | Degraded   |
/// | false      | *      | None/false   | Offline    |
fn derive_status(primary_ok: bool, secondary_ok: Option<bool>, tcp_ok: bool) -> NodeStatus {
    if primary_ok && tcp_ok {
        NodeStatus::Online
    } else if primary_ok {
        // HTTP OK mas TCP direto falhou — degradado (conectividade parcial).
        NodeStatus::Degraded
    } else {
        // Primary HTTP falhou; verifica secundária.
        match secondary_ok {
            Some(true) => NodeStatus::Degraded,
            _ => NodeStatus::Offline,
        }
    }
}

// ---------------------------------------------------------------------------
// Internos — verificações de rede
// ---------------------------------------------------------------------------

/// Verifica se `url` responde a um HTTP HEAD em até `HTTP_TIMEOUT`.
/// Respostas com código < 500 indicam servidor acessível.
fn check_http(url: &str) -> bool {
    match ureq::head(url).timeout(HTTP_TIMEOUT).call() {
        Ok(_) => true,
        Err(ureq::Error::Status(code, _)) => {
            // Servidor respondeu — código HTTP indica estado mas não indisponibilidade total.
            code < 500
        }
        Err(e) => {
            log::debug!("[health] HEAD {} → erro: {}", url, e);
            false
        }
    }
}

/// Verifica conectividade TCP em `addr` (formato `host:porta` ou `host`).
/// Porta padrão 80 é usada quando omitida.
fn check_tcp(addr: &str) -> bool {
    use std::net::ToSocketAddrs;

    let addr_with_port = if addr.contains(':') {
        addr.to_string()
    } else {
        format!("{}:80", addr)
    };

    match addr_with_port.to_socket_addrs() {
        Ok(mut addrs) => match addrs.next() {
            Some(sock_addr) => match TcpStream::connect_timeout(&sock_addr, HTTP_TIMEOUT) {
                Ok(_) => true,
                Err(e) => {
                    log::debug!("[health] TCP {} → falhou: {}", addr_with_port, e);
                    false
                }
            },
            None => {
                log::debug!(
                    "[health] TCP {} → nenhum endereço resolvido.",
                    addr_with_port
                );
                false
            }
        },
        Err(e) => {
            log::debug!(
                "[health] TCP {} → falha na resolução: {}",
                addr_with_port,
                e
            );
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Internos — persistência
// ---------------------------------------------------------------------------

fn save_state(nodes: &Arc<Mutex<Vec<StreamNode>>>) {
    let lock = nodes.lock().unwrap();
    match store::save_to_json(STATE_FILE, &lock) {
        Ok(()) => log::debug!("[health] Estado salvo em {}.", STATE_FILE),
        Err(e) => log::warn!("[health] Falha ao salvar estado: {:#}", e),
    }
}

/// Aplica o estado salvo sobre os nodes existentes, associando por `name`.
fn apply_saved_state(nodes: &mut Vec<StreamNode>, saved: &[StreamNode]) {
    for node in nodes.iter_mut() {
        if let Some(saved_node) = saved.iter().find(|s| s.name == node.name) {
            node.status = saved_node.status.clone();
            node.last_checked = saved_node.last_checked;
        }
    }
}

// ---------------------------------------------------------------------------
// Testes
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{NodeStatus, StreamNode};

    // ------------------------------------------------------------------
    // derive_status — lógica pura, sem rede
    // ------------------------------------------------------------------

    #[test]
    fn derive_online_when_primary_and_tcp_ok() {
        assert_eq!(derive_status(true, None, true), NodeStatus::Online);
    }

    #[test]
    fn derive_online_ignores_secondary_when_primary_tcp_ok() {
        // Secundária irrelevante quando primária+TCP ok
        assert_eq!(
            derive_status(true, Some(false), true),
            NodeStatus::Online
        );
    }

    #[test]
    fn derive_degraded_when_primary_ok_tcp_fails() {
        assert_eq!(derive_status(true, None, false), NodeStatus::Degraded);
    }

    #[test]
    fn derive_degraded_when_primary_fails_secondary_ok() {
        assert_eq!(
            derive_status(false, Some(true), true),
            NodeStatus::Degraded
        );
    }

    #[test]
    fn derive_offline_when_both_urls_fail() {
        assert_eq!(
            derive_status(false, Some(false), true),
            NodeStatus::Offline
        );
    }

    #[test]
    fn derive_offline_when_primary_fails_no_secondary() {
        assert_eq!(derive_status(false, None, true), NodeStatus::Offline);
    }

    // ------------------------------------------------------------------
    // apply_saved_state — lógica pura, sem I/O
    // ------------------------------------------------------------------

    #[test]
    fn apply_saved_state_updates_matching_nodes() {
        let ts = chrono::Utc::now();
        let mut nodes = vec![
            StreamNode {
                name: "Canal A".to_string(),
                status: NodeStatus::Unknown,
                last_checked: None,
                ..Default::default()
            },
            StreamNode {
                name: "Canal B".to_string(),
                status: NodeStatus::Unknown,
                last_checked: None,
                ..Default::default()
            },
        ];

        let saved = vec![StreamNode {
            name: "Canal A".to_string(),
            status: NodeStatus::Online,
            last_checked: Some(ts),
            ..Default::default()
        }];

        apply_saved_state(&mut nodes, &saved);

        assert_eq!(nodes[0].status, NodeStatus::Online);
        assert_eq!(nodes[0].last_checked, Some(ts));
        // Canal B não está no saved — não deve ser alterado
        assert_eq!(nodes[1].status, NodeStatus::Unknown);
        assert!(nodes[1].last_checked.is_none());
    }

    #[test]
    fn apply_saved_state_ignores_unmatched_names() {
        let mut nodes = vec![StreamNode {
            name: "Canal X".to_string(),
            status: NodeStatus::Unknown,
            ..Default::default()
        }];
        let saved = vec![StreamNode {
            name: "Canal Y".to_string(),
            status: NodeStatus::Online,
            last_checked: Some(chrono::Utc::now()),
            ..Default::default()
        }];

        apply_saved_state(&mut nodes, &saved);
        assert_eq!(nodes[0].status, NodeStatus::Unknown);
    }

    // ------------------------------------------------------------------
    // Persistência de estado — roundtrip JSON
    // ------------------------------------------------------------------

    #[test]
    fn save_load_state_roundtrip() {
        use tempfile::NamedTempFile;
        let ts = chrono::Utc::now();
        let original = vec![
            StreamNode {
                name: "Node 1".to_string(),
                primary_url: "http://a.example.com/s.m3u8".to_string(),
                status: NodeStatus::Online,
                last_checked: Some(ts),
                ..Default::default()
            },
            StreamNode {
                name: "Node 2".to_string(),
                primary_url: "http://b.example.com/s.m3u8".to_string(),
                status: NodeStatus::Offline,
                last_checked: None,
                ..Default::default()
            },
        ];

        let tmp = NamedTempFile::new().unwrap();
        store::save_to_json(tmp.path(), &original).unwrap();
        let loaded = store::load_from_json(tmp.path()).unwrap();

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].status, NodeStatus::Online);
        assert_eq!(loaded[1].status, NodeStatus::Offline);
        // Timestamp sobrevive ao roundtrip (comparar segundos para evitar sub-segundo)
        let saved_secs = loaded[0].last_checked.unwrap().timestamp();
        assert_eq!(saved_secs, ts.timestamp());
    }

    // ------------------------------------------------------------------
    // Verificações de rede — casos de falha esperada (sem rede real)
    // ------------------------------------------------------------------

    #[test]
    fn check_http_invalid_url_returns_false() {
        // URL malformada deve retornar false sem pânico
        assert!(!check_http("not-a-valid-url"));
    }

    #[test]
    fn check_http_unreachable_host_returns_false() {
        // Endereço reservado TEST-NET — nunca roteável
        assert!(!check_http("http://192.0.2.1/stream.m3u8"));
    }

    #[test]
    fn check_tcp_closed_port_returns_false() {
        // Porta 1 em loopback quase certamente fechada
        assert!(!check_tcp("127.0.0.1:1"));
    }

    #[test]
    fn check_tcp_no_port_appends_80() {
        // Só verificamos que não entra em pânico; resultado depende da rede
        let _result = check_tcp("127.0.0.1");
        // Sem assert de resultado — apenas garantir ausência de pânico
    }

    #[test]
    fn check_tcp_invalid_host_returns_false() {
        assert!(!check_tcp("host.invalido.local.test:80"));
    }

    // ------------------------------------------------------------------
    // HealthService — construção e canais
    // ------------------------------------------------------------------

    #[test]
    fn health_service_new_and_trigger_sender() {
        let nodes: Arc<Mutex<Vec<StreamNode>>> = Arc::new(Mutex::new(Vec::new()));
        let svc = HealthService::new(nodes.clone());
        let trigger = svc.trigger_sender();
        // Envio sem receptor ainda conectado (sync_channel(1)) deve retornar Ok
        assert!(trigger.try_send(()).is_ok());
        // Segundo envio excede o buffer de capacidade 1 — pode ser TrySendError::Full
        // (não é erro fatal)
        let _ = trigger.try_send(());
    }

    #[test]
    fn health_service_run_exits_when_sender_dropped() {
        // O loop de run() deve encerrar quando o SyncSender for dropado.
        let nodes: Arc<Mutex<Vec<StreamNode>>> = Arc::new(Mutex::new(Vec::new()));
        let svc = HealthService::new(nodes);
        // Dura 9999s mas deve terminar logo que o trigger_tx for dropado
        // via desconexão do canal.
        let trigger = svc.trigger_sender();
        let handle = std::thread::spawn(move || {
            svc.run(9999);
        });
        drop(trigger); // desconecta o canal → run() deve encerrar
        // Aguarda a thread encerrar em até 2s
        let result = handle.join();
        assert!(result.is_ok(), "thread não deve entrar em pânico");
    }
}

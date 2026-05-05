/// Lógica de fallback automático entre URL primária e secundária de um `StreamNode`.
///
/// A função principal é [`play_with_fallback`], que toca a URL primária e, em
/// caso de erro de reprodução relatado pelo MPV, tenta automaticamente a URL
/// secundária. O estado do nó (`status`) é atualizado conforme a tentativa.
use crate::types::{NodeStatus, StreamNode};

// Importações que exigem linkagem com mpv.lib — excluídas em builds de teste
// para que os testes de lógica pura possam rodar sem o DLL instalado.
#[cfg(not(test))]
use anyhow::{anyhow, Result};
#[cfg(not(test))]
use libmpv::events::Event;
#[cfg(not(test))]
use crate::player::MpvPlayer;

// ---------------------------------------------------------------------------
// Lógica pura de seleção de URL — testável sem MPV
// ---------------------------------------------------------------------------

/// Retorna a URL a tentar dado o número da tentativa atual:
/// - tentativa `0` → URL primária
/// - tentativa `1` → URL secundária (se existir)
/// - tentativa `≥2` ou secundária ausente → `None` (sem mais URLs)
pub fn next_url<'a>(node: &'a StreamNode, attempt: usize) -> Option<&'a str> {
    match attempt {
        0 => Some(node.primary_url.as_str()),
        1 => node.secondary_url.as_deref(),
        _ => None,
    }
}

/// Atualiza o `NodeStatus` do nó de acordo com qual tentativa está ativa:
/// - tentativa `0` → `Online` (URL primária OK)
/// - tentativa `1` → `Degraded` (usando URL secundária)
/// - tentativa `≥2` → `Offline` (ambas falharam)
pub fn update_status(node: &mut StreamNode, attempt: usize) {
    node.status = match attempt {
        0 => NodeStatus::Online,
        1 => NodeStatus::Degraded,
        _ => NodeStatus::Offline,
    };
}

// ---------------------------------------------------------------------------
// Engine de fallback com loop de eventos MPV
// ---------------------------------------------------------------------------

/// Inicia reprodução do `node` com fallback automático para URL secundária.
///
/// Fluxo:
/// 1. Toca a URL primária do node.
#[cfg(not(test))]
/// 2. Monitora eventos MPV em loop bloqueante (até Ctrl+C ou shutdown).
/// 3. Se o MPV reportar um erro de reprodução (`Some(Err(_))`):
///    - Se `secondary_url` estiver definida e ainda não foi tentada, troca
///      automaticamente e marca `status = Degraded`.
///    - Se não há mais URLs, marca `status = Offline` e retorna `Err`.
/// 4. Se MPV reportar shutdown ou parada normal, encerra com `Ok`.
///
/// O processo fica bloqueado nesta função até o encerramento. Para uso em UI,
/// chame em thread separada.
pub fn play_with_fallback(node: &mut StreamNode, player: &MpvPlayer) -> Result<()> {
    let mut attempt: usize = 0;

    // Selecionar a URL inicial (primária) e iniciar reprodução
    let first_url = next_url(node, attempt)
        .ok_or_else(|| anyhow!("Node '{}' não possui URL primária", node.name))?
        .to_owned();

    log::info!(
        "[fallback] Iniciando reprodução: '{}' → {}",
        node.name,
        first_url
    );
    player.play(&first_url)?;
    update_status(node, attempt);

    // Criar contexto de eventos para monitorar o estado do MPV
    let mut ev_ctx = player.create_event_context();

    loop {
        // Aguardar até 2 segundos por um evento; None = timeout ou EOF normal
        match ev_ctx.wait_event(2.0) {
            // -----------------------------------------------------------------
            // Erro reportado pelo MPV (falha de stream, rede, codec, etc.)
            // -----------------------------------------------------------------
            Some(Err(e)) => {
                let url_usada = next_url(node, attempt)
                    .unwrap_or("<desconhecida>")
                    .to_owned();
                log::warn!(
                    "[fallback] Erro na reprodução de '{}' (tentativa {}): {}",
                    url_usada,
                    attempt,
                    e
                );

                // Tentar próxima URL
                attempt += 1;
                match next_url(node, attempt) {
                    Some(next) => {
                        let next = next.to_owned();
                        log::info!(
                            "[fallback] Tentando URL de fallback (tentativa {}): {}",
                            attempt,
                            next
                        );
                        update_status(node, attempt);
                        player.play(&next)?;
                    }
                    None => {
                        update_status(node, attempt);
                        log::error!(
                            "[fallback] Todas as URLs de '{}' falharam.",
                            node.name
                        );
                        return Err(anyhow!(
                            "Todas as URLs do endpoint '{}' falharam",
                            node.name
                        ));
                    }
                }
            }

            // -----------------------------------------------------------------
            // MPV encerrou (usuário fechou a janela, Ctrl+C no terminal MPV, etc.)
            // -----------------------------------------------------------------
            Some(Ok(Event::Shutdown)) => {
                log::info!("[fallback] MPV shutdown recebido — encerrando.");
                break;
            }

            // -----------------------------------------------------------------
            // Arquivo/stream terminou (stop, quit ou redirect)
            // -----------------------------------------------------------------
            Some(Ok(Event::EndFile(reason))) => {
                log::info!(
                    "[fallback] EndFile recebido (reason={}). Encerrando loop.",
                    reason
                );
                break;
            }

            // -----------------------------------------------------------------
            // Timeout (nenhum evento dentro do intervalo) — continuar aguardando
            // -----------------------------------------------------------------
            None => {
                // Nenhum evento; continua monitorando
            }

            // Outros eventos (StartFile, FileLoaded, PropertyChange, etc.) — ignorar
            Some(Ok(_)) => {}
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Testes unitários — sem dependência de MPV
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(primary: &str, secondary: Option<&str>) -> StreamNode {
        StreamNode {
            name: "Teste".to_string(),
            primary_url: primary.to_string(),
            secondary_url: secondary.map(|s| s.to_string()),
            ip_address: None,
            status: NodeStatus::Unknown,
            last_checked: None,
        }
    }

    // --- next_url ---

    #[test]
    fn next_url_attempt0_returns_primary() {
        let node = make_node("http://primary.example.com/s.m3u8", None);
        assert_eq!(
            next_url(&node, 0),
            Some("http://primary.example.com/s.m3u8")
        );
    }

    #[test]
    fn next_url_attempt1_returns_secondary_when_present() {
        let node = make_node(
            "http://primary.example.com/s.m3u8",
            Some("http://secondary.example.com/s.m3u8"),
        );
        assert_eq!(
            next_url(&node, 1),
            Some("http://secondary.example.com/s.m3u8")
        );
    }

    #[test]
    fn next_url_attempt1_returns_none_when_secondary_absent() {
        let node = make_node("http://primary.example.com/s.m3u8", None);
        assert_eq!(next_url(&node, 1), None);
    }

    #[test]
    fn next_url_attempt2_always_none() {
        let node = make_node(
            "http://primary.example.com/s.m3u8",
            Some("http://secondary.example.com/s.m3u8"),
        );
        assert_eq!(next_url(&node, 2), None);
        assert_eq!(next_url(&node, 99), None);
    }

    // --- update_status ---

    #[test]
    fn update_status_attempt0_is_online() {
        let mut node = make_node("http://primary.example.com/s.m3u8", None);
        update_status(&mut node, 0);
        assert_eq!(node.status, NodeStatus::Online);
    }

    #[test]
    fn update_status_attempt1_is_degraded() {
        let mut node = make_node("http://primary.example.com/s.m3u8", None);
        update_status(&mut node, 1);
        assert_eq!(node.status, NodeStatus::Degraded);
    }

    #[test]
    fn update_status_attempt2_is_offline() {
        let mut node = make_node("http://primary.example.com/s.m3u8", None);
        update_status(&mut node, 2);
        assert_eq!(node.status, NodeStatus::Offline);
    }

    #[test]
    fn update_status_high_attempt_is_offline() {
        let mut node = make_node("http://primary.example.com/s.m3u8", None);
        update_status(&mut node, 10);
        assert_eq!(node.status, NodeStatus::Offline);
    }

    // --- Cenários de sequência de fallback ---

    #[test]
    fn fallback_sequence_with_secondary() {
        // Simula: primária OK → falha → secundária → falha → Offline
        let node = make_node(
            "http://primary.example.com/s.m3u8",
            Some("http://secondary.example.com/s.m3u8"),
        );

        // Tentativa 0: primária
        assert_eq!(next_url(&node, 0), Some("http://primary.example.com/s.m3u8"));
        // Tentativa 1: secundária
        assert_eq!(
            next_url(&node, 1),
            Some("http://secondary.example.com/s.m3u8")
        );
        // Tentativa 2: sem mais URLs
        assert_eq!(next_url(&node, 2), None);
    }

    #[test]
    fn fallback_sequence_without_secondary() {
        // Simula: primária OK → falha → sem secundária → Offline imediato
        let node = make_node("http://primary.example.com/s.m3u8", None);

        assert_eq!(next_url(&node, 0), Some("http://primary.example.com/s.m3u8"));
        assert_eq!(next_url(&node, 1), None);
    }
}

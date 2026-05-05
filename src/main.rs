// Oculta o console no Windows em builds release (quando integrado à UI egui).
// Em modo CLI (debug), mantém o console visível para logs.
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

pub mod app;
pub mod fallback;
pub mod health;
pub mod store;
pub mod types;

// O módulo `player` requer linkagem com mpv-1.dll.
// É excluído em builds de teste para permitir testes unitários sem o DLL.
#[cfg(not(test))]
pub mod player;

#[cfg(not(test))]
use fallback::play_with_fallback;
#[cfg(not(test))]
use player::MpvPlayer;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    log::info!("mx-player iniciando...");

    let args: Vec<String> = std::env::args().collect();

    match parse_args(&args) {
        Some(CliCommand::PlayEndpoint(name)) => {
            #[cfg(not(test))]
            if let Err(e) = run_endpoint(&name) {
                log::error!("{:#}", e);
                std::process::exit(1);
            }
            #[cfg(test)]
            let _ = name; // satisfazer o compilador em modo teste
        }
        Some(CliCommand::ListEndpoints) => {
            if let Err(e) = list_endpoints() {
                log::error!("{:#}", e);
                std::process::exit(1);
            }
        }
        None => {
            // Sem argumentos CLI → iniciar interface gráfica egui
            if let Err(e) = run_gui() {
                log::error!("Falha ao iniciar GUI: {:#}", e);
                std::process::exit(1);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing de argumentos da linha de comando
// ---------------------------------------------------------------------------

enum CliCommand {
    /// `--endpoint <nome>` — reproduz o endpoint com o nome fornecido.
    PlayEndpoint(String),
    /// `--list` — lista os endpoints disponíveis.
    ListEndpoints,
}

fn parse_args(args: &[String]) -> Option<CliCommand> {
    let mut iter = args.iter().skip(1); // pular o nome do executável
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--endpoint" | "-e" => {
                if let Some(name) = iter.next() {
                    return Some(CliCommand::PlayEndpoint(name.clone()));
                } else {
                    eprintln!("Erro: --endpoint requer um argumento <nome>");
                    return None;
                }
            }
            "--list" | "-l" => return Some(CliCommand::ListEndpoints),
            "--help" | "-h" => return None,
            other => {
                eprintln!("Argumento desconhecido: {}", other);
                return None;
            }
        }
    }
    None
}

fn print_usage(bin: &str) {
    eprintln!(
        "Uso: {} --endpoint <nome>\n\
         \n\
         Opções:\n\
         \t--endpoint, -e <nome>  Reproduz o endpoint com o nome especificado\n\
         \t--list, -l             Lista os endpoints disponíveis em data/\n\
         \t--help, -h             Exibe esta ajuda\n\
         \n\
         Fontes carregadas de data/sources.csv ou data/sources.example.json",
        bin
    );
}

// ---------------------------------------------------------------------------
// Comandos
// ---------------------------------------------------------------------------

/// Carrega as fontes disponíveis tentando data/sources.csv primeiro,
/// com fallback para data/sources.example.json.
fn load_sources() -> anyhow::Result<Vec<types::StreamNode>> {
    let candidates = ["data/sources.csv", "data/sources.example.json"];
    for path in &candidates {
        if std::path::Path::new(path).exists() {
            log::info!("Carregando sources de: {}", path);
            return store::load(path);
        }
    }
    anyhow::bail!(
        "Nenhum arquivo de sources encontrado. \
        Crie data/sources.csv ou data/sources.example.json"
    )
}

/// Executa o CLI de reprodução: carrega sources, localiza o endpoint pelo
/// nome e chama `play_with_fallback` bloqueando até o encerramento.
#[cfg(not(test))]
fn run_endpoint(name: &str) -> anyhow::Result<()> {
    let mut nodes = load_sources()?;

    let node = nodes
        .iter_mut()
        .find(|n| n.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Endpoint '{}' não encontrado. Use --list para ver os disponíveis.",
                name
            )
        })?;

    log::info!(
        "Endpoint encontrado: '{}' → {}",
        node.name,
        node.primary_url
    );

    let player = MpvPlayer::new()?;
    play_with_fallback(node, &player)
}

/// Lista todos os endpoints disponíveis com suas URLs e status.
fn list_endpoints() -> anyhow::Result<()> {
    let nodes = load_sources()?;
    if nodes.is_empty() {
        println!("Nenhum endpoint cadastrado.");
        return Ok(());
    }
    println!("{:<30} {:<50} {}", "Nome", "URL Primária", "Secundária");
    println!("{}", "-".repeat(100));
    for n in &nodes {
        println!(
            "{:<30} {:<50} {}",
            n.name,
            n.primary_url,
            n.secondary_url.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

/// Inicia a interface gráfica egui (caminho padrão quando sem argumentos CLI).
fn run_gui() -> anyhow::Result<()> {
    use std::sync::{Arc, Mutex};

    // Tentar carregar sources; em caso de falha, iniciar com lista vazia
    let mut nodes = load_sources().unwrap_or_else(|e| {
        log::warn!("Sem sources carregados: {:#}. Iniciando com lista vazia.", e);
        Vec::new()
    });

    // Restaurar último status salvo antes de exibir a UI (evita exibir \"Unknown\" no início)
    health::restore_saved_state(&mut nodes);

    // Estado compartilhado entre a UI (App) e o HealthService
    let shared_nodes = Arc::new(Mutex::new(nodes.clone()));

    // Spawnar serviço de health check em thread dedicada (intervalo 300s = 5 min)
    let health_service = health::HealthService::new(shared_nodes.clone());
    let health_trigger = health_service.trigger_sender();
    std::thread::spawn(move || {
        health_service.run(300);
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0_f32, 650.0_f32])
            .with_min_inner_size([700.0_f32, 400.0_f32])
            .with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        "MX Player",
        options,
        Box::new(move |cc| {
            Box::new(app::App::new(cc, nodes, shared_nodes, Some(health_trigger)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {}", e))
}

// ---------------------------------------------------------------------------
// Testes unitários do parsing de CLI
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn args(slice: &[&str]) -> Vec<String> {
        slice.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_endpoint_long() {
        let a = args(&["mx-player", "--endpoint", "Canal 1"]);
        let cmd = parse_args(&a);
        assert!(matches!(cmd, Some(CliCommand::PlayEndpoint(ref n)) if n == "Canal 1"));
    }

    #[test]
    fn parse_endpoint_short() {
        let a = args(&["mx-player", "-e", "Canal 2"]);
        let cmd = parse_args(&a);
        assert!(matches!(cmd, Some(CliCommand::PlayEndpoint(ref n)) if n == "Canal 2"));
    }

    #[test]
    fn parse_list_long() {
        let a = args(&["mx-player", "--list"]);
        assert!(matches!(parse_args(&a), Some(CliCommand::ListEndpoints)));
    }

    #[test]
    fn parse_list_short() {
        let a = args(&["mx-player", "-l"]);
        assert!(matches!(parse_args(&a), Some(CliCommand::ListEndpoints)));
    }

    #[test]
    fn parse_help_returns_none() {
        let a = args(&["mx-player", "--help"]);
        assert!(parse_args(&a).is_none());
    }

    #[test]
    fn parse_missing_endpoint_value_returns_none() {
        let a = args(&["mx-player", "--endpoint"]);
        assert!(parse_args(&a).is_none());
    }

    #[test]
    fn parse_unknown_arg_returns_none() {
        let a = args(&["mx-player", "--foobar"]);
        assert!(parse_args(&a).is_none());
    }

    #[test]
    fn parse_no_args_returns_none() {
        let a = args(&["mx-player"]);
        assert!(parse_args(&a).is_none());
    }
}


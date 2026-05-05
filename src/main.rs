// Hide console window on Windows release builds
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

// TODO (Fase 1): adicionar módulos conforme implementação
// mod types;
// mod store;
// mod player;
// mod health;
// mod app;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    log::info!("mx-player iniciando...");
    // TODO (Fase 1): CLI — carregar sources e invocar libmpv para testar fallback
    // TODO (Fase 2): inicializar eframe/egui com painel de player integrado
    // TODO (Fase 3): lançar health-check tokio em background
    println!("mx-player: stub — veja TASKS.md para o roadmap do MVP");
}


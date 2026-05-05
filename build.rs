fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=MPV_LIB_PATH");

    // Integração libmpv: configura caminho de busca para libmpv.dll.a (import library MinGW)
    // A crate `libmpv` emite `cargo:rustc-link-lib=mpv`; este build.rs indica ONDE encontrá-la.
    //
    // Pré-requisitos (Windows):
    //   1. Baixar mpv-dev MinGW de https://sourceforge.net/projects/mpv-player-windows/files/libmpv/
    //      Confirmar que os binários usam msvcrt (não ucrt) para compatibilidade com Win7 SP1
    //   2. Extrair e definir MPV_LIB_PATH apontando para o subdiretório que contém libmpv.dll.a
    //      Exemplo: $env:MPV_LIB_PATH = "C:\mpv-dev\lib64"
    //   3. Garantir que mpv-1.dll esteja junto ao .exe no deploy final
    //
    // Win7 compatibility note: mpv <= 0.36 com build MinGW não requer ucrt.
    // Testar com: mpv-dev-20231231-git-<hash>-x86_64.7z (verificar data/hash sem ucrt).
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        match std::env::var("MPV_LIB_PATH") {
            Ok(mpv_lib_path) => {
                println!("cargo:rustc-link-search=native={}", mpv_lib_path);
            }
            Err(_) => {
                // Fallback: tenta caminhos convencionais; emite warning para orientar o dev
                println!("cargo:rustc-link-search=native=C:\\mpv-dev\\lib64");
                println!("cargo:rustc-link-search=native=C:\\mpv-dev\\lib");
                println!(
                    "cargo:warning=MPV_LIB_PATH não definido. \
                    Defina-a apontando para o diretório com libmpv.dll.a. \
                    Execute scripts/check_env.ps1 para diagnóstico completo."
                );
            }
        }
    }

    // Recurso de versão do executável Windows (manifesto, ícone, strings de versão)
    if cfg!(target_os = "windows") {
        let mut res = winres::WindowsResource::new();
        res.set("ProductName", "MX Player");
        res.set("FileDescription", "Monitor e player nativo de streams HLS");
        res.set("LegalCopyright", "Copyright © 2026 Matheus Rodacki");
        let _ = res.compile();
    }
}

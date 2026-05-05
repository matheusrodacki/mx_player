fn main() {
    if cfg!(target_os = "windows") {
        let mut res = winres::WindowsResource::new();
        res.set("ProductName", "MX Player");
        res.set("FileDescription", "Monitor e player nativo de streams HLS");
        res.set("LegalCopyright", "Copyright © 2026 Matheus Rodacki");
        let _ = res.compile();
    }
}

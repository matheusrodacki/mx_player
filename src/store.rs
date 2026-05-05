use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

use crate::types::{NodeStatus, StreamNode};

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

/// Carrega uma lista de nós a partir de um arquivo JSON.
///
/// O arquivo deve conter um array JSON de objetos `StreamNode`.
/// Exemplo: `data/sources.example.json`.
pub fn load_from_json<P: AsRef<Path>>(path: P) -> Result<Vec<StreamNode>> {
    let path = path.as_ref();
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Falha ao ler JSON: {}", path.display()))?;
    let nodes: Vec<StreamNode> = serde_json::from_str(&content)
        .with_context(|| format!("Falha ao parsear JSON: {}", path.display()))?;
    Ok(nodes)
}

/// Persiste uma lista de nós em um arquivo JSON.
pub fn save_to_json<P: AsRef<Path>>(path: P, nodes: &[StreamNode]) -> Result<()> {
    let path = path.as_ref();
    let content = serde_json::to_string_pretty(nodes)
        .context("Falha ao serializar nós para JSON")?;
    std::fs::write(path, content)
        .with_context(|| format!("Falha ao escrever JSON: {}", path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// CSV
// ---------------------------------------------------------------------------

/// Registro intermediário para leitura de CSV.
/// Campos opcionais são representados como `String` e convertidos para
/// `Option<String>` (vazio → `None`) ao construir o `StreamNode`.
#[derive(Debug, Deserialize)]
struct CsvRecord {
    name: String,
    primary_url: String,
    #[serde(default)]
    secondary_url: String,
    #[serde(default)]
    ip_address: String,
}

impl From<CsvRecord> for StreamNode {
    fn from(r: CsvRecord) -> StreamNode {
        StreamNode {
            name: r.name,
            primary_url: r.primary_url,
            secondary_url: if r.secondary_url.trim().is_empty() {
                None
            } else {
                Some(r.secondary_url)
            },
            ip_address: if r.ip_address.trim().is_empty() {
                None
            } else {
                Some(r.ip_address)
            },
            status: NodeStatus::Unknown,
            last_checked: None,
        }
    }
}

/// Carrega uma lista de nós a partir de um arquivo CSV.
///
/// Colunas esperadas (cabeçalho obrigatório):
/// `name,primary_url,secondary_url,ip_address`
///
/// As colunas `secondary_url` e `ip_address` são opcionais — células em
/// branco são tratadas como ausentes (`None`).
pub fn load_from_csv<P: AsRef<Path>>(path: P) -> Result<Vec<StreamNode>> {
    let path = path.as_ref();
    let mut reader = csv::Reader::from_path(path)
        .with_context(|| format!("Falha ao abrir CSV: {}", path.display()))?;

    let mut nodes = Vec::new();
    for (i, result) in reader.deserialize::<CsvRecord>().enumerate() {
        let record = result
            .with_context(|| format!("Falha ao parsear linha {} do CSV: {}", i + 2, path.display()))?;
        nodes.push(StreamNode::from(record));
    }
    Ok(nodes)
}

// ---------------------------------------------------------------------------
// Auto-detect
// ---------------------------------------------------------------------------

/// Detecta o formato pelo sufixo do arquivo e delega para `load_from_json`
/// ou `load_from_csv` conforme apropriado.
pub fn load<P: AsRef<Path>>(path: P) -> Result<Vec<StreamNode>> {
    let path = path.as_ref();
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => load_from_json(path),
        Some("csv") => load_from_csv(path),
        other => anyhow::bail!(
            "Formato de arquivo não suportado: {:?}. Use .json ou .csv",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Testes
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn make_json_file(content: &str) -> NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(".json")
            .tempfile()
            .unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    fn make_csv_file(content: &str) -> NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(".csv")
            .tempfile()
            .unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    // ---- JSON ----

    #[test]
    fn load_json_full_node() {
        let json = r#"[
            {
                "name": "Canal 1",
                "primary_url": "http://host/stream.m3u8",
                "secondary_url": "http://backup/stream.m3u8",
                "ip_address": "10.0.0.1",
                "status": "online",
                "last_checked": null
            }
        ]"#;
        let f = make_json_file(json);
        let nodes = load_from_json(f.path()).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].name, "Canal 1");
        assert_eq!(nodes[0].secondary_url.as_deref(), Some("http://backup/stream.m3u8"));
        assert_eq!(nodes[0].status, NodeStatus::Online);
    }

    #[test]
    fn load_json_optional_fields_absent() {
        let json = r#"[
            { "name": "Canal 2", "primary_url": "http://host/live.m3u8" }
        ]"#;
        let f = make_json_file(json);
        let nodes = load_from_json(f.path()).unwrap();
        assert_eq!(nodes.len(), 1);
        assert!(nodes[0].secondary_url.is_none());
        assert!(nodes[0].ip_address.is_none());
        assert_eq!(nodes[0].status, NodeStatus::Unknown);
    }

    #[test]
    fn load_json_multiple_nodes() {
        let json = r#"[
            { "name": "A", "primary_url": "http://a.com/s.m3u8" },
            { "name": "B", "primary_url": "http://b.com/s.m3u8" },
            { "name": "C", "primary_url": "http://c.com/s.m3u8" }
        ]"#;
        let f = make_json_file(json);
        let nodes = load_from_json(f.path()).unwrap();
        assert_eq!(nodes.len(), 3);
    }

    #[test]
    fn save_and_reload_json() {
        let original = vec![
            StreamNode {
                name: "Save Test".to_string(),
                primary_url: "http://test.com/s.m3u8".to_string(),
                secondary_url: None,
                ip_address: Some("192.168.0.1".to_string()),
                status: NodeStatus::Offline,
                last_checked: None,
            },
        ];
        let f = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        save_to_json(f.path(), &original).unwrap();
        let reloaded = load_from_json(f.path()).unwrap();
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded[0].name, "Save Test");
        assert_eq!(reloaded[0].status, NodeStatus::Offline);
    }

    // ---- CSV ----

    #[test]
    fn load_csv_full_row() {
        let csv = "name,primary_url,secondary_url,ip_address\n\
                   Canal 1,http://host/s.m3u8,http://bk/s.m3u8,10.0.0.1\n";
        let f = make_csv_file(csv);
        let nodes = load_from_csv(f.path()).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].name, "Canal 1");
        assert_eq!(nodes[0].secondary_url.as_deref(), Some("http://bk/s.m3u8"));
        assert_eq!(nodes[0].ip_address.as_deref(), Some("10.0.0.1"));
    }

    #[test]
    fn load_csv_empty_optional_fields() {
        let csv = "name,primary_url,secondary_url,ip_address\n\
                   Canal 2,http://host/live.m3u8,,\n";
        let f = make_csv_file(csv);
        let nodes = load_from_csv(f.path()).unwrap();
        assert_eq!(nodes.len(), 1);
        assert!(nodes[0].secondary_url.is_none());
        assert!(nodes[0].ip_address.is_none());
        assert_eq!(nodes[0].status, NodeStatus::Unknown);
    }

    #[test]
    fn load_csv_multiple_rows() {
        let csv = "name,primary_url,secondary_url,ip_address\n\
                   A,http://a.com/s.m3u8,,\n\
                   B,http://b.com/s.m3u8,http://b2.com/s.m3u8,\n\
                   C,http://c.com/s.m3u8,,172.16.0.5\n";
        let f = make_csv_file(csv);
        let nodes = load_from_csv(f.path()).unwrap();
        assert_eq!(nodes.len(), 3);
        assert!(nodes[0].secondary_url.is_none());
        assert_eq!(nodes[1].secondary_url.as_deref(), Some("http://b2.com/s.m3u8"));
        assert_eq!(nodes[2].ip_address.as_deref(), Some("172.16.0.5"));
    }

    // ---- Auto-detect ----

    #[test]
    fn auto_detect_json() {
        let json = r#"[{ "name": "X", "primary_url": "http://x.com/s.m3u8" }]"#;
        let f = make_json_file(json);
        let nodes = load(f.path()).unwrap();
        assert_eq!(nodes.len(), 1);
    }

    #[test]
    fn auto_detect_csv() {
        let csv = "name,primary_url,secondary_url,ip_address\nX,http://x.com/s.m3u8,,\n";
        let f = make_csv_file(csv);
        let nodes = load(f.path()).unwrap();
        assert_eq!(nodes.len(), 1);
    }

    #[test]
    fn auto_detect_unsupported_extension_errors() {
        let result = load("/tmp/sources.txt");
        assert!(result.is_err());
    }
}

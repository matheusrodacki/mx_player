use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Estado de saúde de um nó de stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Online,
    Degraded,
    Offline,
    Unknown,
}

impl Default for NodeStatus {
    fn default() -> Self {
        NodeStatus::Unknown
    }
}

/// Representa um endpoint de stream com URL primária e, opcionalmente, URL secundária de fallback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamNode {
    /// Nome descritivo do endpoint (ex: "Canal 1 - SP").
    pub name: String,
    /// URL principal do stream (ex: `http://host/hls/stream.m3u8`).
    pub primary_url: String,
    /// URL de fallback usada se a primária falhar.
    pub secondary_url: Option<String>,
    /// Endereço IP/host do servidor (para health check TCP).
    pub ip_address: Option<String>,
    /// Estado atual do nó (atualizado pelo health check).
    #[serde(default)]
    pub status: NodeStatus,
    /// Última vez que o health check foi executado para este nó.
    pub last_checked: Option<DateTime<Utc>>,
}

impl Default for StreamNode {
    fn default() -> Self {
        StreamNode {
            name: String::new(),
            primary_url: String::new(),
            secondary_url: None,
            ip_address: None,
            status: NodeStatus::Unknown,
            last_checked: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_status_default_is_unknown() {
        assert_eq!(NodeStatus::default(), NodeStatus::Unknown);
    }

    #[test]
    fn stream_node_default_has_empty_urls() {
        let node = StreamNode::default();
        assert!(node.name.is_empty());
        assert!(node.primary_url.is_empty());
        assert!(node.secondary_url.is_none());
        assert!(node.ip_address.is_none());
        assert!(node.last_checked.is_none());
        assert_eq!(node.status, NodeStatus::Unknown);
    }

    #[test]
    fn stream_node_roundtrip_json() {
        let node = StreamNode {
            name: "Canal Teste".to_string(),
            primary_url: "http://example.com/stream.m3u8".to_string(),
            secondary_url: Some("http://backup.example.com/stream.m3u8".to_string()),
            ip_address: Some("192.168.1.10".to_string()),
            status: NodeStatus::Online,
            last_checked: None,
        };
        let json = serde_json::to_string(&node).unwrap();
        let parsed: StreamNode = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, node.name);
        assert_eq!(parsed.primary_url, node.primary_url);
        assert_eq!(parsed.secondary_url, node.secondary_url);
        assert_eq!(parsed.status, NodeStatus::Online);
    }

    #[test]
    fn node_status_serializes_snake_case() {
        let json = serde_json::to_string(&NodeStatus::Degraded).unwrap();
        assert_eq!(json, "\"degraded\"");
        let json = serde_json::to_string(&NodeStatus::Online).unwrap();
        assert_eq!(json, "\"online\"");
    }
}

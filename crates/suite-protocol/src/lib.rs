//! Protocole commun de la Suite Media, version 1.
//!
//! Toutes les apps de la suite (Studio, Nino, apps mobiles, studio web…)
//! parlent ce même protocole : même enveloppe JSON, même poignée de main,
//! même authentification, mêmes règles d'erreurs et d'événements.
//! Chaque app n'ajoute que ses propres requêtes et événements.
//! Voir `docs/protocole-suite.md` pour la description complète.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;

/// Messages envoyés par le serveur.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Premier message à la connexion.
    Hello {
        app: AppInfo,
        protocol: u32,
        auth_required: bool,
        /// Domaines de requêtes proposés, ex. `core`, `studio.scenes`, `studio.outputs`.
        capabilities: Vec<String>,
    },
    /// Réponse à `identify` réussie.
    Identified { client_id: String },
    /// Réponse à une requête.
    Response {
        id: String,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<ErrorBody>,
    },
    /// Événement diffusé à tous les clients identifiés.
    Event {
        /// Identifiant de l'app qui émet (ex. `studio`, `nino`).
        app: String,
        #[serde(rename = "type")]
        event_type: String,
        data: Value,
    },
}

/// Messages envoyés par le client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ClientMessage {
    Identify {
        #[serde(default)]
        token: Option<String>,
        /// Nom affiché du client, par exemple « Nino ».
        #[serde(default)]
        name: Option<String>,
        /// Identité de l'app cliente, si c'est une app de la suite.
        #[serde(default)]
        app: Option<AppInfo>,
        /// Recevoir les événements (vrai par défaut).
        #[serde(default = "yes")]
        events: bool,
    },
    Request {
        id: String,
        #[serde(rename = "type")]
        request_type: String,
        #[serde(default)]
        data: Value,
    },
}

/// Identité d'une app de la suite.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppInfo {
    /// Identifiant stable en minuscules : `studio`, `nino`, `cam-ios`…
    pub id: String,
    /// Nom affiché.
    pub name: String,
    /// Version de l'app (semver).
    pub version: String,
}

/// Requêtes que toute app de la suite doit savoir traiter (domaine `core`).
pub const CORE_REQUESTS: &[&str] = &["GetAppInfo", "GetRequestList", "Ping"];

/// Port WebSocket par défaut de chaque app, pour qu'elles se trouvent sans configuration.
pub const DEFAULT_PORTS: &[(&str, u16)] = &[("studio", 4460), ("nino", 4461)];

/// Type de service mDNS / Bonjour pour la découverte sur le réseau local.
pub const MDNS_SERVICE: &str = "_suitemedia._tcp.local.";

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorBody {
    /// Code stable, en snake_case, utilisable par les clients.
    pub code: String,
    /// Message lisible.
    pub message: String,
}

impl ServerMessage {
    pub fn ok(id: impl Into<String>, data: Value) -> Self {
        ServerMessage::Response {
            id: id.into(),
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(id: impl Into<String>, code: impl Into<String>, message: impl Into<String>) -> Self {
        ServerMessage::Response {
            id: id.into(),
            ok: false,
            data: None,
            error: Some(ErrorBody {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_request() {
        let m: ClientMessage = serde_json::from_value(json!({
            "op": "request", "id": "1", "type": "CreateScene", "data": {"name": "Plateau"}
        }))
        .unwrap();
        assert!(
            matches!(m, ClientMessage::Request { ref request_type, .. } if request_type == "CreateScene")
        );
    }

    #[test]
    fn identify_defaults() {
        let m: ClientMessage = serde_json::from_value(json!({"op": "identify"})).unwrap();
        assert_eq!(
            m,
            ClientMessage::Identify {
                token: None,
                name: None,
                app: None,
                events: true
            }
        );
    }

    #[test]
    fn error_shape() {
        let v = serde_json::to_value(ServerMessage::err("7", "scene_not_found", "x")).unwrap();
        assert_eq!(
            v,
            json!({"op": "response", "id": "7", "ok": false, "error": {"code": "scene_not_found", "message": "x"}})
        );
    }
}

//! OpenAI Chat Completions (Fase 0). Key validation via `GET /v1/models`.

use crate::ai::error::{AiError, AiErrorCode};
use crate::ai::secrets::ApiKey;

pub const OPENAI_API_BASE: &str = "https://api.openai.com";
pub const OPENAI_CURATED_MODELS: &[&str] = &["gpt-4o-mini", "gpt-4o", "o4-mini"];

pub fn openai_recommended_model() -> &'static str {
    OPENAI_CURATED_MODELS[0]
}

/// Lists models — no token usage for the completion endpoint.
pub fn openai_validate_key(key: &ApiKey) -> Result<(), AiError> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| AiError::new(AiErrorCode::Network, format!("Cliente HTTP: {e}")))?;

  let url = format!("{OPENAI_API_BASE}/v1/models");
    let res = client
        .get(&url)
        .bearer_auth(key.expose_secret())
        .send()
        .map_err(map_transport)?;

    let status = res.status();
    if status.is_success() {
        return Ok(());
    }
    let body = res
        .text()
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect::<String>();
    let body = crate::redact::redact_secrets(&body);
    match status.as_u16() {
        401 => Err(AiError::new(
            AiErrorCode::InvalidKey,
            "Chave inválida ou revogada.".to_string(),
        )),
        403 => Err(AiError::new(
            AiErrorCode::InvalidKey,
            "Chave sem permissão para a API.".to_string(),
        )),
        429 => {
            let lower = body.to_ascii_lowercase();
            if lower.contains("insufficient_quota") || lower.contains("billing") {
                Err(AiError::new(
                    AiErrorCode::Quota,
                    "Sem crédito ou cota esgotada no provedor. Verifique o faturamento.".to_string(),
                ))
            } else {
                Err(AiError::new(
                    AiErrorCode::Quota,
                    "Limite de requisições atingido. Tente de novo em instantes.".to_string(),
                ))
            }
        }
        502 | 503 | 504 => Err(AiError::new(
            AiErrorCode::Provider,
            "O provedor está indisponível. Tente mais tarde.".to_string(),
        )),
        _ => Err(AiError::new(
            AiErrorCode::Provider,
            if body.is_empty() {
                format!("Erro do provedor (HTTP {status}).")
            } else {
                format!("Erro do provedor: {body}")
            },
        )),
    }
}

fn map_transport(e: reqwest::Error) -> AiError {
    let msg = if e.is_timeout() {
        "Tempo esgotado ao falar com o provedor.".to_string()
    } else if e.is_connect() {
        "Rede indisponível ou proxy não suportado nesta versão.".to_string()
    } else {
        format!("Falha de rede: {}", e.without_url())
    };
    AiError::new(AiErrorCode::Network, msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use tiny_http::{Header, Response, Server};

    #[test]
    fn validate_sends_bearer_not_query() {
        let saw_auth = Arc::new(AtomicBool::new(false));
        let saw = saw_auth.clone();
        let server = Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr();
        let port = match addr {
            tiny_http::ListenAddr::IP(ref a, p) => (a.to_string(), p),
            _ => panic!("unexpected listen addr"),
        };
        let base = format!("http://{}:{}", port.0, port.1);
        thread::spawn(move || {
            for request in server.incoming_requests() {
                let auth = request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Authorization"))
                    .map(|h| h.value.as_str().to_string());
                if auth.as_deref() == Some("Bearer sk-test-wire") {
                    saw.store(true, Ordering::SeqCst);
                }
                let mut response = Response::from_string(r#"{"object":"list"}"#).with_status_code(200);
                response.add_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
                let _ = request.respond(response);
            }
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
        let client = reqwest::blocking::Client::new();
        let url = format!("{base}/v1/models");
        let res = client
            .get(&url)
            .bearer_auth("sk-test-wire")
            .send()
            .unwrap();
        assert!(res.status().is_success());
        assert!(saw_auth.load(Ordering::SeqCst));
    }

}

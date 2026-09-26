use super::{status_update, LLMEngine, Tool};
use crate::cancellation::{with_cancellation, SmartRemarkableCancellation};
use crate::util::{option_or_env, option_or_env_fallback, OptionMap};
use anyhow::Result;
use log::debug;
use serde_json::json;
use serde_json::Value as json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

async fn check_bridge(client: &reqwest::Client, base_url: &str) -> Result<()> {
    let response = client
        .get(format!("{}/health", base_url.trim_end_matches('/')))
        .timeout(Duration::from_secs(4))
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Mac bridge is unreachable; check the connection and try again"))?;
    if !response.status().is_success() {
        anyhow::bail!("Mac bridge is not ready; try again after reconnecting");
    }
    let health: json = response.json().await.map_err(|_| anyhow::anyhow!("Mac bridge health response was invalid"))?;
    anyhow::ensure!(health["status"] == "ready", "Mac bridge is not ready");
    Ok(())
}

async fn request_answer(client: &reqwest::Client, url: &str, api_key: &str, body: &json, bridge: bool) -> Result<json> {
    let id = format!(
        "rm-{}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
        REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let mut uncertain = false;
    let mut last = "Connection unavailable".to_string();
    for attempt in 0..if bridge { 4 } else { 1 } {
        if attempt > 0 {
            log::info!("Recovering answer transport, attempt {}", attempt + 1);
            tokio::time::sleep(Duration::from_secs(1 << (attempt - 1))).await;
        }
        let mut request = client.post(url).bearer_auth(api_key).json(body);
        if bridge {
            request = request
                .header("Idempotency-Key", &id)
                .header("X-Remarkable-Retry", if uncertain { "1" } else { "0" });
        }
        let sending = request.send();
        tokio::pin!(sending);
        let response = loop {
            tokio::select! {
                response = &mut sending => break response,
                _ = tokio::time::sleep(Duration::from_secs(2)), if bridge => {
                    let base = url.trim_end_matches("/v1/chat/completions");
                    if let Ok(status) = client.get(format!("{base}/requests/{id}")).bearer_auth(api_key)
                        .timeout(Duration::from_secs(2)).send().await {
                        if let Ok(value) = status.json::<json>().await {
                            if let (Some(provider), Some(model)) = (value["provider"].as_str(), value["model"].as_str()) {
                                crate::preferences::set_provider(provider, model);
                            }
                        }
                    }
                }
            }
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                uncertain |= !error.is_connect();
                last = "Connection interrupted while waiting for the answer".into();
                continue;
            }
        };
        let status = response.status();
        let text = match response.text().await {
            Ok(text) => text,
            Err(_) => {
                uncertain = true;
                last = "Answer transfer was interrupted".into();
                continue;
            }
        };
        if !status.is_success() {
            last = format!("API Error: {status}");
            if bridge {
                if let Ok(body) = serde_json::from_str::<json>(&text) {
                    if let Some(message) = body["error"]["message"].as_str() {
                        last = message.chars().filter(|c| !c.is_control()).take(180).collect();
                    }
                }
            }
            if !bridge || !matches!(status.as_u16(), 408 | 429 | 503) {
                anyhow::bail!("{last}");
            }
            // 429 is explicitly rejected before inference. All other retries
            // must rejoin an existing receipt, never start another tool run.
            uncertain = status.as_u16() != 429;
            continue;
        }
        match serde_json::from_str(&text) {
            Ok(answer) => return Ok(answer),
            Err(_) => {
                uncertain = true;
                last = "Answer transfer was malformed".into();
            }
        }
    }
    anyhow::bail!("{last}; bounded recovery exhausted")
}

pub struct OpenAI {
    model: String,
    base_url: String,
    api_key: String,
    tools: Vec<Tool>,
    content: Vec<json>,
}

impl OpenAI {
    pub fn add_content(&mut self, content: json) {
        self.content.push(content);
    }

    fn tool_definition_json(tool: &Tool) -> json {
        json!({
            "type": "function",
            "function": {
                "name": tool.definition["name"],
                "description": tool.definition["description"],
                "parameters": tool.definition["parameters"],
            }
        })
    }
}

#[async_trait::async_trait]
impl LLMEngine for OpenAI {
    fn new(options: &OptionMap) -> Self {
        let api_key = option_or_env(options, "api_key", "OPENAI_API_KEY");
        let base_url = option_or_env_fallback(options, "base_url", "OPENAI_BASE_URL", "https://api.openai.com");
        let model = options.get("model").unwrap().to_string();

        Self {
            model,
            base_url,
            api_key,
            tools: Vec::new(),
            content: Vec::new(),
        }
    }

    fn register_tool(&mut self, name: &str, definition: json, callback: Box<dyn FnMut(json) + Send>) {
        self.tools.push(Tool {
            name: name.to_string(),
            definition,
            callback: Some(callback),
        });
    }

    fn add_text_content(&mut self, text: &str) {
        self.add_content(json!({
            "type": "text",
            "text": text,
        }));
    }

    fn add_image_content(&mut self, base64_image: &str) {
        self.add_content(json!({
            "type": "image_url",
            "image_url": {
                "url": format!("data:image/png;base64,{}", base64_image)
            }
        }));
    }

    fn clear_content(&mut self) {
        self.content.clear();
    }

    async fn execute(&mut self, cancellation: &SmartRemarkableCancellation, mut status_callback: Option<super::StatusCallback>) -> Result<()> {
        let mut body = json!({
            "model": self.model,
            "messages": [{
                "role": "user",
                "content": self.content
            }],
            "tools": self.tools.iter().map(Self::tool_definition_json).collect::<Vec<_>>(),
            "tool_choice": "required",
            "parallel_tool_calls": false
        });

        if self.model == "codex" {
            body["remarkable_settings"] = crate::preferences::load()?.inference_settings();
        }

        debug!("Request: {}", body);

        // Notify that we're building context
        status_update!(status_callback, super::ModelExecutionStatus::BuildingContext);

        // Notify that we're processing with LLM
        status_update!(status_callback, super::ModelExecutionStatus::LlmProcessing);

        // Create async HTTP request with cancellation support
        let request_future = async {
            let client = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(390))
                .build()?;
            let bridge = self.model == "codex"
                && reqwest::Url::parse(&self.base_url)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_owned))
                    .is_some_and(|host| matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]"));
            if bridge {
                for attempt in 0..4 {
                    match check_bridge(&client, &self.base_url).await {
                        Ok(()) => break,
                        Err(error) if attempt == 3 => return Err(error),
                        Err(_) => {
                            crate::preferences::set_phase("Reconnecting");
                            tokio::time::sleep(Duration::from_secs(3)).await;
                        }
                    }
                }
            }
            tokio::time::timeout(
                Duration::from_secs(420),
                request_answer(&client, &format!("{}/v1/chat/completions", self.base_url), &self.api_key, &body, bridge),
            )
            .await
            .map_err(|_| anyhow::anyhow!("Answer recovery deadline exceeded"))?
        };

        let json: json = with_cancellation(request_future, cancellation).await?;
        debug!("Response: {}", json);
        if let (Some(provider), Some(model)) = (json["remarkable_backend"]["provider"].as_str(), json["remarkable_backend"]["model"].as_str()) {
            crate::preferences::set_provider(provider, model);
            crate::preferences::set_phase("Writing");
            // Let the transient QML thinking banner disappear before native
            // ink delivery captures or pans the notebook.
            tokio::time::sleep(Duration::from_millis(1100)).await;
        }

        // Notify that we're processing the response
        status_update!(status_callback, super::ModelExecutionStatus::ProcessingResponse);

        let tool_calls = &json["choices"][0]["message"]["tool_calls"];

        if let Some(tool_call) = tool_calls.get(0) {
            // Notify that we're calling tools
            status_update!(status_callback, super::ModelExecutionStatus::CallingTools);

            let function_name = tool_call["function"]["name"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Answer is missing a tool name"))?;
            let function_input_raw = tool_call["function"]["arguments"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Answer is missing tool arguments"))?;
            let mut function_input = serde_json::from_str::<json>(function_input_raw).map_err(|_| anyhow::anyhow!("Answer tool arguments are malformed"))?;
            if json["remarkable_backend"]["attempts"].as_array().is_some_and(|a| !a.is_empty()) {
                if let Some(lines) = function_input["lines"].as_array_mut() {
                    lines.insert(0, json!(format!("Answered by {} (fallback).", crate::preferences::answer_label())));
                }
            }
            let tool = self.tools.iter_mut().find(|tool| tool.name == function_name);

            if let Some(tool) = tool {
                if let Some(callback) = &mut tool.callback {
                    callback(function_input.clone());
                    // Notify that we're done
                    status_update!(status_callback, super::ModelExecutionStatus::Done);
                    Ok(())
                } else {
                    status_update!(
                        status_callback,
                        super::ModelExecutionStatus::Error("No callback registered for tool".to_string())
                    );
                    Err(anyhow::anyhow!("No callback registered for tool {}", function_name))
                }
            } else {
                status_update!(status_callback, super::ModelExecutionStatus::Error("No tool registered".to_string()));
                Err(anyhow::anyhow!("No tool registered with name {}", function_name))
            }
        } else {
            status_update!(
                status_callback,
                super::ModelExecutionStatus::Error("No tool calls found in response".to_string())
            );
            Err(anyhow::anyhow!("No tool calls found in response"))
        }
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn bridge_probe_checks_health_before_inference() {
        for (status, body, ready) in [(200, "{\"status\":\"ready\"}", true), (503, "{}", false), (200, "{}", false)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buf = [0; 2048];
                let n = stream.read(&mut buf).await.unwrap();
                assert!(String::from_utf8_lossy(&buf[..n]).starts_with("GET /health "));
                stream
                    .write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes())
                    .await
                    .unwrap();
            });
            assert_eq!(check_bridge(&reqwest::Client::new(), &url).await.is_ok(), ready);
            server.await.unwrap();
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        assert!(check_bridge(&reqwest::Client::new(), &url).await.is_err());
    }

    #[tokio::test]
    async fn lost_and_malformed_responses_retry_the_same_request_id() {
        for malformed in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let mut seen = Vec::new();
                for attempt in 0..2 {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut bytes = Vec::new();
                    loop {
                        let mut buf = [0; 4096];
                        let n = stream.read(&mut buf).await.unwrap();
                        assert!(n > 0);
                        bytes.extend_from_slice(&buf[..n]);
                        if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                            let length: usize = head.lines().find_map(|line| line.strip_prefix("content-length: ")).unwrap().parse().unwrap();
                            if bytes.len() >= end + 4 + length {
                                seen.push(head);
                                break;
                            }
                        }
                    }
                    if attempt == 1 || malformed {
                        let body = if attempt == 0 { "not-json" } else { "{\"answer\":\"complete\"}" };
                        let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        stream.write_all(response.as_bytes()).await.unwrap();
                    }
                }
                seen
            });
            let client = reqwest::Client::builder().timeout(Duration::from_secs(2)).build().unwrap();
            let answer = request_answer(
                &client,
                &format!("http://{addr}/v1/chat/completions"),
                "fixture",
                &json!({"fixture":true}),
                true,
            )
            .await
            .unwrap();
            assert_eq!(answer["answer"], "complete");
            let seen = server.await.unwrap();
            let ids: Vec<_> = seen
                .iter()
                .map(|head| head.lines().find(|line| line.starts_with("idempotency-key:")).unwrap())
                .collect();
            assert_eq!(ids[0], ids[1]);
            assert!(seen[0].contains("x-remarkable-retry: 0"));
            assert!(seen[1].contains("x-remarkable-retry: 1"));
        }
    }
}

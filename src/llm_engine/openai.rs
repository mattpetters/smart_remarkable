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
        let response = match request.send().await {
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
            body["remarkable_settings"] = serde_json::to_value(crate::preferences::load()?)?;
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
                .timeout(Duration::from_secs(210))
                .build()?;
            let bridge = self.model == "codex"
                && reqwest::Url::parse(&self.base_url)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_owned))
                    .is_some_and(|host| matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]"));
            tokio::time::timeout(
                Duration::from_secs(240),
                request_answer(&client, &format!("{}/v1/chat/completions", self.base_url), &self.api_key, &body, bridge),
            )
            .await
            .map_err(|_| anyhow::anyhow!("Answer recovery deadline exceeded"))?
        };

        let json: json = with_cancellation(request_future, cancellation).await?;
        debug!("Response: {}", json);

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
            let function_input = serde_json::from_str::<json>(function_input_raw).map_err(|_| anyhow::anyhow!("Answer tool arguments are malformed"))?;
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

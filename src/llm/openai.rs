use crate::{
    config::LlmConfig,
    error::{AppError, Result},
    llm::provider::{LlmProvider, LlmResponse, PromptContext},
};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{env, time::Duration};

pub struct OpenAiProvider {
    client: Client,
    config: LlmConfig,
}

impl OpenAiProvider {
    pub fn new(config: LlmConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .expect("openai client should build");
        Self { client, config }
    }

    fn api_key(&self) -> Result<String> {
        env::var(&self.config.openai.api_key_env).map_err(|_| {
            AppError::Llm(format!(
                "{} is not configured",
                self.config.openai.api_key_env
            ))
        })
    }
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
}

#[derive(Debug, Serialize)]
struct Message {
    role: &'static str,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: AssistantMessage,
}

#[derive(Debug, Deserialize)]
struct AssistantMessage {
    content: String,
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn generate(&self, prompt: &PromptContext) -> Result<LlmResponse> {
        let endpoint = format!(
            "{}/chat/completions",
            self.config.openai.base_url.trim_end_matches('/')
        );
        let request = ChatRequest {
            model: self.config.model.clone(),
            messages: vec![
                Message {
                    role: "system",
                    content: "You are a research synthesis engine. Return two sections labeled `Summary:` and `Inference:`. Ground the response in provided evidence.".into(),
                },
                Message {
                    role: "user",
                    content: format!(
                        "Workspace: {}\nObjective: {}\nEvidence:\n{}",
                        prompt.workspace_name,
                        prompt.objective,
                        prompt.evidence.join("\n\n")
                    ),
                },
            ],
        };
        let response = self
            .client
            .post(endpoint)
            .bearer_auth(self.api_key()?)
            .json(&request)
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(AppError::Llm(format!(
                "openai returned status {}: {}",
                status, body
            )));
        }

        let parsed: ChatResponse = serde_json::from_str(&body)?;
        let content = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| AppError::Llm("openai returned no choices".into()))?
            .message
            .content;
        let mut summary = String::new();
        let mut inference = String::new();
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("Summary:") {
                summary.push_str(rest.trim());
            } else if let Some(rest) = line.strip_prefix("Inference:") {
                inference.push_str(rest.trim());
            }
        }
        if summary.is_empty() {
            summary = content.clone();
        }
        if inference.is_empty() {
            inference = "Inference was not separated by the provider response.".into();
        }
        Ok(LlmResponse { summary, inference })
    }
}

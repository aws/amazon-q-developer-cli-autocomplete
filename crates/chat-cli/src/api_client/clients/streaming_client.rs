use std::sync::{
    Arc,
    Mutex,
};

use amzn_codewhisperer_streaming_client::Client as CodewhispererStreamingClient;
use aws_types::request_id::RequestId;
use tracing::{
    debug,
    error,
};

use super::shared::{
    bearer_sdk_config,
    stalled_stream_protection_config,
};
use crate::api_client::interceptor::opt_out::OptOutInterceptor;
use crate::api_client::model::{
    ChatResponseStream,
    ConversationState,
};
use crate::api_client::{
    ApiClientError,
    Endpoint,
};
use crate::auth::builder_id::BearerResolver;
use crate::aws_common::{
    UserAgentOverrideInterceptor,
    app_name,
};
use crate::database::{
    AuthProfile,
    Database,
};

mod inner {
    use std::sync::{
        Arc,
        Mutex,
    };

    use amzn_codewhisperer_streaming_client::Client as CodewhispererStreamingClient;

    use crate::api_client::model::ChatResponseStream;

    #[derive(Clone, Debug)]
    pub enum Inner {
        Codewhisperer(CodewhispererStreamingClient),
        Mock(Arc<Mutex<std::vec::IntoIter<Vec<ChatResponseStream>>>>),
    }
}

#[derive(Clone, Debug)]
pub struct StreamingClient {
    inner: inner::Inner,
    profile: Option<AuthProfile>,
}

impl StreamingClient {
    pub async fn new(database: &mut Database) -> Result<Self, ApiClientError> {
        Self::new_codewhisperer_client(database, &Endpoint::load_codewhisperer(database)).await
    }

    pub fn mock(events: Vec<Vec<ChatResponseStream>>) -> Self {
        Self {
            inner: inner::Inner::Mock(Arc::new(Mutex::new(events.into_iter()))),
            profile: None,
        }
    }

    pub async fn new_codewhisperer_client(
        database: &mut Database,
        endpoint: &Endpoint,
    ) -> Result<Self, ApiClientError> {
        let conf_builder: amzn_codewhisperer_streaming_client::config::Builder =
            (&bearer_sdk_config(database, endpoint).await).into();
        let conf = conf_builder
            .http_client(crate::aws_common::http_client::client())
            .interceptor(OptOutInterceptor::new(database))
            .interceptor(UserAgentOverrideInterceptor::new())
            .bearer_token_resolver(BearerResolver)
            .app_name(app_name())
            .endpoint_url(endpoint.url())
            .stalled_stream_protection(stalled_stream_protection_config())
            .build();
        let inner = inner::Inner::Codewhisperer(CodewhispererStreamingClient::from_conf(conf));

        let profile = match database.get_auth_profile() {
            Ok(profile) => profile,
            Err(err) => {
                error!("Failed to get auth profile: {err}");
                None
            },
        };

        Ok(Self { inner, profile })
    }

    pub async fn send_message(
        &self,
        conversation_state: ConversationState,
    ) -> Result<SendMessageOutput, ApiClientError> {
        debug!("Sending conversation: {:#?}", conversation_state);
        let ConversationState {
            conversation_id,
            user_input_message,
            history,
        } = conversation_state;

        match &self.inner {
            inner::Inner::Codewhisperer(client) => {
                let model_id_opt: Option<String> = user_input_message.model_id.clone();
                let conversation_state = amzn_codewhisperer_streaming_client::types::ConversationState::builder()
                    .set_conversation_id(conversation_id)
                    .current_message(
                        amzn_codewhisperer_streaming_client::types::ChatMessage::UserInputMessage(
                            user_input_message.into(),
                        ),
                    )
                    .chat_trigger_type(amzn_codewhisperer_streaming_client::types::ChatTriggerType::Manual)
                    .set_history(
                        history
                            .map(|v| v.into_iter().map(|i| i.try_into()).collect::<Result<Vec<_>, _>>())
                            .transpose()?,
                    )
                    .build()
                    .expect("building conversation_state should not fail");
                let response = client
                    .generate_assistant_response()
                    .conversation_state(conversation_state)
                    .set_profile_arn(self.profile.as_ref().map(|p| p.arn.clone()))
                    .send()
                    .await;

                match response {
                    Ok(resp) => Ok(SendMessageOutput::Codewhisperer(resp)),
                    Err(e) => {
                        let status_code = e.raw_response().map(|res| res.status().as_u16());

                        let is_quota_breach = status_code.is_some_and(|status| status == 429);
                        let is_context_window_overflow = e.as_service_error().is_some_and(|err| {
                            matches!(err, err if err.meta().code() == Some("ValidationException")
                                && err.meta().message() == Some("Input is too long."))
                        });

                        let is_model_unavailable = model_id_opt.is_some()
                            && status_code.is_some_and(|status| status == 500)
                            && e.as_service_error().is_some_and(|err| {
                                err.meta().message()
                                == Some("Encountered unexpectedly high load when processing the request, please try again.")
                            });
                        let is_monthly_limit_err = e
                            .raw_response()
                            .and_then(|resp| resp.body().bytes())
                            .and_then(|bytes| match String::from_utf8(bytes.to_vec()) {
                                Ok(s) => Some(s.contains("MONTHLY_REQUEST_COUNT")),
                                Err(_) => None,
                            })
                            .unwrap_or(false);

                        if is_quota_breach {
                            Err(ApiClientError::QuotaBreach {
                                message: "quota has reached its limit",
                                status_code,
                            })
                        } else if is_context_window_overflow {
                            Err(ApiClientError::ContextWindowOverflow { status_code })
                        } else if is_model_unavailable {
                            let request_id = e
                                .as_service_error()
                                .and_then(|err| err.meta().request_id())
                                .map(|s| s.to_string());
                            Err(ApiClientError::ModelOverloadedError {
                                request_id,
                                status_code,
                            })
                        } else if is_monthly_limit_err {
                            Err(ApiClientError::MonthlyLimitReached { status_code })
                        } else {
                            Err(e.into())
                        }
                    },
                }
            },
            inner::Inner::Mock(events) => {
                let mut new_events = events.lock().unwrap().next().unwrap_or_default().clone();
                new_events.reverse();
                Ok(SendMessageOutput::Mock(new_events))
            },
        }
    }
}

#[derive(Debug)]
pub enum SendMessageOutput {
    Codewhisperer(
        amzn_codewhisperer_streaming_client::operation::generate_assistant_response::GenerateAssistantResponseOutput,
    ),
    Mock(Vec<ChatResponseStream>),
}

impl SendMessageOutput {
    pub fn request_id(&self) -> Option<&str> {
        match self {
            SendMessageOutput::Codewhisperer(output) => output.request_id(),
            SendMessageOutput::Mock(_) => None,
        }
    }

    pub async fn recv(&mut self) -> Result<Option<ChatResponseStream>, ApiClientError> {
        match self {
            SendMessageOutput::Codewhisperer(output) => Ok(output
                .generate_assistant_response_response
                .recv()
                .await?
                .map(|s| s.into())),
            SendMessageOutput::Mock(vec) => Ok(vec.pop()),
        }
    }
}

impl RequestId for SendMessageOutput {
    fn request_id(&self) -> Option<&str> {
        match self {
            SendMessageOutput::Codewhisperer(output) => output.request_id(),
            SendMessageOutput::Mock(_) => Some("<mock-request-id>"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_client::model::{
        AssistantResponseMessage,
        ChatMessage,
        UserInputMessage,
    };

    #[tokio::test]
    async fn create_clients() {
        let mut database = Database::new().await.unwrap();
        let endpoint = Endpoint::load_codewhisperer(&database);

        let _ = StreamingClient::new(&mut database).await;
        let _ = StreamingClient::new_codewhisperer_client(&mut database, &endpoint).await;
    }

    #[tokio::test]
    async fn test_mock() {
        let client = StreamingClient::mock(vec![vec![
            ChatResponseStream::AssistantResponseEvent {
                content: "Hello!".to_owned(),
            },
            ChatResponseStream::AssistantResponseEvent {
                content: " How can I".to_owned(),
            },
            ChatResponseStream::AssistantResponseEvent {
                content: " assist you today?".to_owned(),
            },
        ]]);
        let mut output = client
            .send_message(ConversationState {
                conversation_id: None,
                user_input_message: UserInputMessage {
                    images: None,
                    content: "Hello".into(),
                    user_input_message_context: None,
                    user_intent: None,
                    model_id: Some("model".to_owned()),
                },
                history: None,
            })
            .await
            .unwrap();

        let mut output_content = String::new();
        while let Some(ChatResponseStream::AssistantResponseEvent { content }) = output.recv().await.unwrap() {
            output_content.push_str(&content);
        }
        assert_eq!(output_content, "Hello! How can I assist you today?");
    }

    #[ignore]
    #[tokio::test]
    async fn assistant_response() {
        let mut database = Database::new().await.unwrap();
        let client = StreamingClient::new(&mut database).await.unwrap();
        let mut response = client
            .send_message(ConversationState {
                conversation_id: None,
                user_input_message: UserInputMessage {
                    images: None,
                    content: "How about rustc?".into(),
                    user_input_message_context: None,
                    user_intent: None,
                    model_id: Some("model".to_owned()),
                },
                history: Some(vec![
                    ChatMessage::UserInputMessage(UserInputMessage {
                        images: None,
                        content: "What language is the linux kernel written in, and who wrote it?".into(),
                        user_input_message_context: None,
                        user_intent: None,
                        model_id: None,
                    }),
                    ChatMessage::AssistantResponseMessage(AssistantResponseMessage {
                        content: "It is written in C by Linus Torvalds.".into(),
                        message_id: None,
                        tool_uses: None,
                    }),
                ]),
            })
            .await
            .unwrap();

        while let Some(event) = response.recv().await.unwrap() {
            println!("{:?}", event);
        }
    }
}

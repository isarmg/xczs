use super::{
    Request, Response, Server, apply_app_error,
    operation_registry::{
        OPERATION_ID_HEADER, OPERATION_STATE_HEADER, OperationOutcome, apply_operation_outcome,
        apply_unknown, set_operation_headers,
    },
    problem::{ApiError, ErrorCode, OperationProblemContext, RecoveryAdvice, render_problem},
    protocol::{OperationPublicState, UploadPublicState},
    set_private_no_store, status_error, upload,
};
use crate::{app_error::AppError, http_logger::HttpLogger, request_context::RequestContext};

use anyhow::Result;
use axum::{Router, extract::ConnectInfo, response::IntoResponse};
use http::{Method, StatusCode, header::CONTENT_LENGTH};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tower::ServiceExt;

mod files;
mod request;
mod routes;

pub(in crate::server) use request::MutationProgress;
use request::RequestProfile;

impl Server {
    pub fn http_service(
        self: Arc<Self>,
        handle: xcss::server_runtime::RuntimeHandle,
    ) -> anyhow::Result<tower::util::BoxCloneSyncService<Request, Response, Infallible>> {
        let router = routes::assemble(self.clone(), handle)?;
        let trusted_proxies =
            xcss::client_ip::TrustedProxies::parse(&self.content.args.trusted_proxies)?;
        let service = tower::service_fn(move |request| {
            let server = self.clone();
            let router = router.clone();
            async move { server.process_request(request, router).await }
        });
        let correlated = axum::Router::new()
            .fallback_service(service)
            .layer(axum::middleware::from_fn_with_state(
                trusted_proxies,
                xcss::client_ip::middleware,
            ))
            .layer(axum::middleware::from_fn_with_state(
                "xczs".to_owned(),
                xcss::server_cli::service_identity_middleware,
            ))
            .layer(axum::middleware::from_fn(
                xcss::server_cli::request_context_middleware,
            ));
        Ok(tower::util::BoxCloneSyncService::new(correlated))
    }

    async fn process_request(
        self: Arc<Self>,
        mut req: Request,
        router: Router,
    ) -> Result<Response, Infallible> {
        let Some(ConnectInfo(addr)) = req
            .extensions()
            .get::<ConnectInfo<std::net::SocketAddr>>()
            .copied()
        else {
            return Ok((http::StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"code":"platform.peer_missing","message":"Socket peer is missing","retryable":false,"details":{}}))).into_response());
        };
        let relative_path = self.resolve_path(req.uri().path());
        let public_asset_request = matches!(req.method(), &Method::GET | &Method::HEAD)
            && relative_path
                .as_deref()
                .is_some_and(|path| self.is_public_asset_path(path));
        let profile = RequestProfile::new(&req, relative_path.as_deref(), public_asset_request);
        let mutation = profile.mutation();
        let auth_log = routes::AuthLog::default();
        req.extensions_mut().insert(auth_log.clone());
        req.extensions_mut().insert(profile.clone());
        req.extensions_mut().insert(mutation.clone());
        if let Some(path) = &relative_path {
            req.extensions_mut().insert(path.clone());
        }
        let mut context = RequestContext::new(&req, addr, &self.content.args.http_logger);
        if let Some(operation_id) = profile.operation_id() {
            self.content.args.http_logger.set_runtime_value(
                context.access_log_mut(),
                "operation_id",
                || operation_id.hyphenated().to_string(),
            );
        }

        if relative_path.is_none() {
            let mut response = Response::default();
            super::status_bad_request(&mut response, "Invalid Path");
            set_private_no_store(&mut response);
            attach_access_log(
                &self.content.args.http_logger,
                &mut context,
                &mut response,
                Some("invalid raw path".into()),
                false,
            );
            return Ok(response);
        }

        // An embedder may retain an Arc<Server> after ServerRuntime shutdown.
        // Reject new work before routing so closed trackers and a stopped
        // maintenance worker cannot acquire fresh filesystem obligations.
        let Some(_request_guard) = self.lifecycle.enter_request().await else {
            let mut res = Response::default();
            if let Some(operation_id) = profile.operation_id() {
                set_operation_headers(&mut res, operation_id, OperationPublicState::Rejected);
                render_problem(
                    &mut res,
                    &ApiError::new(
                        StatusCode::SERVICE_UNAVAILABLE,
                        ErrorCode::SERVER_STOPPING,
                        "Server is shutting down",
                    )
                    .with_recovery(RecoveryAdvice::RetryAfterSeconds(1))
                    .with_operation(OperationProblemContext::new(
                        operation_id.hyphenated().to_string(),
                        OperationPublicState::Rejected,
                        None,
                    )),
                )
                .expect("serializing a fixed operation problem cannot fail");
            } else if let Some(upload) = profile.upload_context() {
                upload::apply_upload_problem(
                    &mut res,
                    upload::UploadErrorContext::new(
                        upload.id,
                        UploadPublicState::NotStarted,
                        Some(upload.length),
                        upload.offset,
                    ),
                    StatusCode::SERVICE_UNAVAILABLE,
                    ErrorCode::SERVER_STOPPING,
                    "Server is shutting down",
                    RecoveryAdvice::RetryAfterSeconds(1),
                )
                .expect("serializing a fixed upload problem cannot fail");
            } else if profile.is_administrator_auth_api() {
                self.render_administrator_auth_error(
                    &mut res,
                    StatusCode::SERVICE_UNAVAILABLE,
                    "service_unavailable",
                    "Server is shutting down",
                    true,
                    Some(1),
                )
                .expect("serializing a fixed administrator auth error cannot fail");
            } else if profile.is_internal_api() {
                render_problem(
                    &mut res,
                    &ApiError::new(
                        StatusCode::SERVICE_UNAVAILABLE,
                        ErrorCode::SERVER_STOPPING,
                        "Server is shutting down",
                    )
                    .with_recovery(RecoveryAdvice::RetryAfterSeconds(1)),
                )
                .expect("serializing a fixed problem response cannot fail");
            } else {
                status_error(
                    &mut res,
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Server is shutting down",
                );
            }
            set_private_no_store(&mut res);
            attach_access_log(
                &self.content.args.http_logger,
                &mut context,
                &mut res,
                Some("server is shutting down".to_string()),
                false,
            );
            return Ok(res);
        };
        let handle = async move {
            let mut response = router
                .oneshot(req)
                .await
                .expect("Axum service is infallible");
            if response
                .extensions_mut()
                .remove::<routes::OmitHeadContentLength>()
                .is_some()
            {
                response.headers_mut().remove(CONTENT_LENGTH);
            }
            if let Some(error) = response.extensions_mut().remove::<routes::BusinessError>() {
                return Err(error
                    .0
                    .lock()
                    .expect("business error lock")
                    .take()
                    .expect("error consumed once"));
            }
            Ok(response)
        };
        let handle_result = if profile.is_upload() {
            handle.await
        } else {
            match tokio::time::timeout(
                Duration::from_secs(self.content.args.request_timeout),
                handle,
            )
            .await
            {
                Ok(result) => result,
                Err(_) => {
                    let mut res = Response::default();
                    if let Some(operation_id) = profile.operation_id() {
                        apply_operation_timeout(&mut res, operation_id, &mutation)
                            .expect("serializing a fixed operation response cannot fail");
                    } else if profile.is_administrator_auth_api() {
                        self.render_administrator_auth_error(
                            &mut res,
                            StatusCode::GATEWAY_TIMEOUT,
                            "request_timeout",
                            "Request timed out",
                            true,
                            None,
                        )
                        .expect("serializing a fixed administrator auth error cannot fail");
                    } else if profile.is_internal_api() {
                        render_problem(
                            &mut res,
                            &ApiError::new(
                                StatusCode::GATEWAY_TIMEOUT,
                                ErrorCode::REQUEST_TIMEOUT,
                                "Request timed out",
                            )
                            .with_recovery(RecoveryAdvice::Retry),
                        )
                        .expect("serializing a fixed problem response cannot fail");
                    } else {
                        status_error(&mut res, StatusCode::GATEWAY_TIMEOUT, "Request timed out");
                    }
                    set_private_no_store(&mut res);
                    if let Some(username) = auth_log.0.lock().expect("auth log lock").as_ref() {
                        self.content
                            .args
                            .http_logger
                            .set_authenticated_user(context.access_log_mut(), username);
                    }
                    attach_access_log(
                        &self.content.args.http_logger,
                        &mut context,
                        &mut res,
                        Some("request time budget exceeded".to_string()),
                        false,
                    );
                    return Ok(res);
                }
            }
        };

        if let Some(username) = auth_log.0.lock().expect("auth log lock").as_ref() {
            self.content
                .args
                .http_logger
                .set_authenticated_user(context.access_log_mut(), username);
        }
        let (mut res, successful_public_asset) = match handle_result {
            Ok(mut res) => {
                if let Some(identity) = res
                    .extensions()
                    .get::<xcss::admin_axum::VerifiedAdministrator>()
                {
                    self.content
                        .args
                        .http_logger
                        .set_authenticated_user(context.access_log_mut(), identity.username());
                }
                if let Some(operation_id) = profile.operation_id()
                    && !res.headers().contains_key(OPERATION_ID_HEADER)
                {
                    let state = if res.status().is_success() {
                        OperationPublicState::Succeeded
                    } else if mutation.outcome_can_be_unknown() {
                        OperationPublicState::Unknown
                    } else {
                        OperationPublicState::Rejected
                    };
                    set_operation_headers(&mut res, operation_id, state);
                }
                let successful_public_asset = profile.is_public_asset()
                    && matches!(res.status(), StatusCode::OK | StatusCode::NOT_MODIFIED);
                let omit_success = profile.omit_success_log() && res.status() == StatusCode::OK;
                attach_access_log(
                    &self.content.args.http_logger,
                    &mut context,
                    &mut res,
                    None,
                    omit_success,
                );
                (res, successful_public_asset)
            }
            Err(err) => {
                let mut res = Response::default();
                let error = AppError::from_anyhow(err);
                if let Some(operation_id) = profile.operation_id() {
                    if mutation.outcome_can_be_unknown() {
                        apply_operation_outcome(
                            &mut res,
                            operation_id,
                            OperationOutcome::uncertain(),
                            false,
                        )
                        .expect("serializing a fixed operation response cannot fail");
                    } else {
                        apply_operation_failure_before_commit(&mut res, operation_id)
                            .expect("serializing a fixed operation response cannot fail");
                    }
                } else if let Some(upload) = profile.upload_context() {
                    let (status, code, detail, recovery, state) =
                        if mutation.outcome_can_be_unknown() {
                            (
                                if error.status() == StatusCode::GATEWAY_TIMEOUT {
                                    StatusCode::GATEWAY_TIMEOUT
                                } else {
                                    StatusCode::INTERNAL_SERVER_ERROR
                                },
                                ErrorCode::UPLOAD_RESULT_UNKNOWN,
                                "Upload result could not be confirmed",
                                RecoveryAdvice::QueryUpload,
                                UploadPublicState::Unknown,
                            )
                        } else {
                            (
                                if error.status() == StatusCode::GATEWAY_TIMEOUT {
                                    StatusCode::REQUEST_TIMEOUT
                                } else {
                                    StatusCode::SERVICE_UNAVAILABLE
                                },
                                if error.status() == StatusCode::GATEWAY_TIMEOUT {
                                    ErrorCode::REQUEST_TIMEOUT
                                } else {
                                    ErrorCode::UPLOAD_PRECOMMIT_FAILED
                                },
                                "Upload stopped before any upload mutation",
                                RecoveryAdvice::Retry,
                                UploadPublicState::NotStarted,
                            )
                        };
                    render_upload_problem(
                        &mut res,
                        status,
                        code,
                        detail,
                        recovery,
                        upload.id,
                        upload.length,
                        upload.offset,
                        state,
                    )
                    .expect("serializing a validated upload problem cannot fail");
                } else if profile.is_administrator_auth_api() {
                    let (status, code, message, retryable) =
                        if error.status() == StatusCode::GATEWAY_TIMEOUT {
                            (
                                StatusCode::GATEWAY_TIMEOUT,
                                "request_timeout",
                                "Request timed out",
                                true,
                            )
                        } else {
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "internal_error",
                                "Administrator authentication request failed",
                                false,
                            )
                        };
                    self.render_administrator_auth_error(
                        &mut res, status, code, message, retryable, None,
                    )
                    .expect("serializing a fixed administrator auth error cannot fail");
                } else if profile.is_internal_api() {
                    render_problem(&mut res, &api_error_from_app_error(&error))
                        .expect("serializing a validated API problem cannot fail");
                } else {
                    apply_app_error(&mut res, &error);
                }
                attach_access_log(
                    &self.content.args.http_logger,
                    &mut context,
                    &mut res,
                    Some(error.to_string()),
                    false,
                );
                (res, false)
            }
        };
        if !successful_public_asset {
            set_private_no_store(&mut res);
        }

        Ok(res)
    }
}

fn attach_access_log(
    logger: &HttpLogger,
    context: &mut RequestContext,
    response: &mut Response,
    error: Option<String>,
    omit_success: bool,
) {
    logger.set_runtime_value(context.access_log_mut(), "status", || {
        response.status().as_u16().to_string()
    });
    if let Some(operation_state) = response
        .headers()
        .get(OPERATION_STATE_HEADER)
        .and_then(|value| value.to_str().ok())
    {
        logger.set_runtime_value(context.access_log_mut(), "operation_state", || {
            operation_state.to_string()
        });
    }
    let body = std::mem::take(response.body_mut());
    let expected_body_bytes = if context.is_head_request()
        || response.status().is_informational()
        || matches!(
            response.status(),
            StatusCode::NO_CONTENT | StatusCode::NOT_MODIFIED
        ) {
        Some(0)
    } else {
        response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
    };
    *response.body_mut() = logger.log_response_body(
        context.access_log().clone(),
        body,
        expected_body_bytes,
        error,
        omit_success,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_upload_problem(
    res: &mut Response,
    status: StatusCode,
    code: ErrorCode,
    detail: impl Into<std::borrow::Cow<'static, str>>,
    recovery: RecoveryAdvice,
    upload_id: uuid::Uuid,
    upload_length: u64,
    upload_offset: Option<u64>,
    state: UploadPublicState,
) -> Result<()> {
    upload::apply_upload_problem(
        res,
        upload::UploadErrorContext::new(upload_id, state, Some(upload_length), upload_offset),
        status,
        code,
        detail,
        recovery,
    )
}

fn api_error_from_app_error(error: &AppError) -> ApiError {
    let code = match error.status() {
        StatusCode::BAD_REQUEST => ErrorCode::INVALID_REQUEST,
        StatusCode::FORBIDDEN => ErrorCode::REQUEST_FORBIDDEN,
        StatusCode::NOT_FOUND => ErrorCode::REQUEST_NOT_FOUND,
        StatusCode::CONFLICT => ErrorCode::REQUEST_CONFLICT,
        StatusCode::GATEWAY_TIMEOUT => ErrorCode::FILESYSTEM_TIMEOUT,
        StatusCode::INSUFFICIENT_STORAGE => ErrorCode::INSUFFICIENT_STORAGE,
        _ => ErrorCode::INTERNAL_ERROR,
    };
    let recovery = if matches!(
        error.status(),
        StatusCode::GATEWAY_TIMEOUT | StatusCode::SERVICE_UNAVAILABLE
    ) {
        RecoveryAdvice::Retry
    } else {
        RecoveryAdvice::None
    };
    ApiError::new(error.status(), code, error.public_message().to_owned()).with_recovery(recovery)
}

fn apply_operation_timeout(
    res: &mut Response,
    operation_id: uuid::Uuid,
    mutation: &MutationProgress,
) -> Result<()> {
    if mutation.outcome_can_be_unknown() {
        return apply_unknown(
            res,
            operation_id,
            StatusCode::GATEWAY_TIMEOUT,
            ErrorCode::OPERATION_RESULT_UNKNOWN,
            "Request timed out after commit dispatch; query the operation status before retrying",
        );
    }

    apply_operation_rejected(
        res,
        operation_id,
        StatusCode::GATEWAY_TIMEOUT,
        ErrorCode::REQUEST_TIMEOUT,
        "Request timed out before filesystem commit; retrying with the same operation ID is safe",
        RecoveryAdvice::Retry,
    )
}

fn apply_operation_failure_before_commit(
    res: &mut Response,
    operation_id: uuid::Uuid,
) -> Result<()> {
    apply_operation_rejected(
        res,
        operation_id,
        StatusCode::INTERNAL_SERVER_ERROR,
        ErrorCode::OPERATION_NOT_COMMITTED,
        "Request failed before filesystem commit; retrying with the same operation ID is safe",
        RecoveryAdvice::Retry,
    )
}

fn apply_operation_rejected(
    res: &mut Response,
    operation_id: uuid::Uuid,
    status: StatusCode,
    code: ErrorCode,
    detail: &'static str,
    recovery: RecoveryAdvice,
) -> Result<()> {
    set_operation_headers(res, operation_id, OperationPublicState::Rejected);
    render_problem(
        res,
        &ApiError::new(status, code, detail)
            .with_recovery(recovery)
            .with_operation(OperationProblemContext::new(
                operation_id.hyphenated().to_string(),
                OperationPublicState::Rejected,
                None,
            )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt as _;

    async fn timeout_problem(progress: &MutationProgress) -> (Response, serde_json::Value) {
        let mut response = Response::default();
        apply_operation_timeout(&mut response, uuid::Uuid::new_v4(), progress).unwrap();
        let body = response.body_mut().collect().await.unwrap().to_bytes();
        let problem = serde_json::from_slice(&body).unwrap();
        (response, problem)
    }

    #[tokio::test]
    async fn precommit_timeout_is_a_retryable_rejection() {
        let progress = MutationProgress::default();
        progress.mark_reserved();

        let (response, problem) = timeout_problem(&progress).await;

        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(response.headers()[OPERATION_STATE_HEADER], "rejected");
        assert_eq!(problem["code"], "request_timeout");
        assert_eq!(problem["recovery"], "retry");
    }

    #[tokio::test]
    async fn detached_commit_timeout_requires_status_query() {
        let progress = MutationProgress::default();
        progress.mark_reserved();
        progress.mark_detached_commit();

        let (response, problem) = timeout_problem(&progress).await;

        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(response.headers()[OPERATION_STATE_HEADER], "unknown");
        assert_eq!(problem["code"], "operation_result_unknown");
        assert_eq!(problem["recovery"], "query_job");
    }

    #[test]
    fn server_stopping_response_populates_access_log_fields() {
        let logger = "$request $remote_addr $status $operation_id $operation_state"
            .parse::<HttpLogger>()
            .unwrap();
        let request = http::Request::builder()
            .method(Method::DELETE)
            .uri("/late-request")
            .body(())
            .unwrap();
        let peer = "192.0.2.10:41234".parse().unwrap();
        let mut context = RequestContext::new(&request, peer, &logger);
        let operation_id = uuid::Uuid::new_v4();
        logger.set_runtime_value(context.access_log_mut(), "operation_id", || {
            operation_id.hyphenated().to_string()
        });
        let mut response = Response::default();
        *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
        set_operation_headers(&mut response, operation_id, OperationPublicState::Rejected);

        attach_access_log(
            &logger,
            &mut context,
            &mut response,
            Some("server is shutting down".to_string()),
            false,
        );

        assert_eq!(
            context.access_log().get("request").map(String::as_str),
            Some("DELETE /late-request HTTP/1.1")
        );
        assert_eq!(
            context.access_log().get("remote_addr").map(String::as_str),
            Some("192.0.2.10")
        );
        assert_eq!(
            context.access_log().get("status").map(String::as_str),
            Some("503")
        );
        assert_eq!(
            context
                .access_log()
                .get("operation_state")
                .map(String::as_str),
            Some("rejected")
        );
    }
}

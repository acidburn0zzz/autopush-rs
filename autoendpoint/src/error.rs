//! Error types and transformations
// TODO: Collpase these into `autopush_common::error`

use crate::headers::vapid::VapidError;
use crate::routers::RouterError;
use actix_web::{
    dev::ServiceResponse,
    error::{JsonPayloadError, PayloadError, ResponseError},
    http::StatusCode,
    middleware::ErrorHandlerResponse,
    HttpResponse, Result,
};
use backtrace::Backtrace; // Sentry uses the backtrace crate, not std::backtrace.
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use std::error::Error;
use std::fmt::{self, Display};
use thiserror::Error;
use validator::{ValidationErrors, ValidationErrorsKind};

use autopush_common::db::error::DbError;
use autopush_common::errors::{ApcError, ApcErrorKind};

/// Common `Result` type.
pub type ApiResult<T> = Result<T, ApiError>;

/// A link for more info on the returned error
const ERROR_URL: &str = "http://autopush.readthedocs.io/en/latest/http.html#error-codes";

/// The main error type.
#[derive(Debug)]
pub struct ApiError {
    pub kind: ApiErrorKind,
    pub backtrace: Backtrace,
}

impl ApiError {
    /// Render a 404 response
    // wrapper during the move. this should switch to autopush-common's impl.
    pub fn render_404<B>(res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
        //TODO: remove unwrap here.
        Ok(autopush_common::errors::render_404(res).unwrap())
    }
}

impl From<ApiError> for ApcError {
    fn from(err: ApiError) -> ApcError {
        ApcError {
            kind: err.kind.into(),
            backtrace: Box::new(err.backtrace),
        }
    }
}

/// The possible errors this application could encounter
#[derive(Debug, Error)]
pub enum ApiErrorKind {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Metrics(#[from] cadence::MetricError),

    #[error(transparent)]
    Validation(#[from] validator::ValidationErrors),

    #[error(transparent)]
    PayloadError(actix_web::Error),

    #[error(transparent)]
    VapidError(#[from] VapidError),

    #[error(transparent)]
    Router(#[from] RouterError),

    #[error(transparent)]
    Jwt(#[from] jsonwebtoken::errors::Error),

    #[error("Error while validating token")]
    TokenHashValidation(#[source] openssl::error::ErrorStack),

    #[error("Error while creating secret")]
    RegistrationSecretHash(#[source] openssl::error::ErrorStack),

    #[error("Error while creating endpoint URL: {0}")]
    EndpointUrl(#[source] autopush_common::errors::ApcError),

    #[error("Database error: {0}")]
    Database(#[from] DbError),

    #[error("Invalid token")]
    InvalidToken,

    #[error("UAID not found")]
    NoUser,

    #[error("No such subscription")]
    NoSubscription,

    /// A specific issue with the encryption headers
    #[error("{0}")]
    InvalidEncryption(String),

    /// Used if the API version given is not v1 or v2
    #[error("Invalid API version")]
    InvalidApiVersion,

    #[error("Missing TTL value")]
    NoTTL,

    #[error("Invalid router type")]
    InvalidRouterType,

    #[error("Invalid router token")]
    InvalidRouterToken,

    #[error("Invalid message ID")]
    InvalidMessageId,

    #[error("Invalid Authentication")]
    InvalidAuthentication,

    #[error("Invalid Local Auth {0}")]
    InvalidLocalAuth(String),

    #[error("General error {0}")]
    General(String),

    #[error("ERROR:Success")]
    LogCheck,
}

impl ApiErrorKind {
    /// Get the associated HTTP status code
    pub fn status(&self) -> StatusCode {
        match self {
            ApiErrorKind::PayloadError(e) => e.as_response_error().status_code(),
            ApiErrorKind::Router(e) => e.status(),

            ApiErrorKind::Validation(_)
            | ApiErrorKind::InvalidEncryption(_)
            | ApiErrorKind::NoTTL
            | ApiErrorKind::InvalidRouterType
            | ApiErrorKind::InvalidRouterToken
            | ApiErrorKind::InvalidMessageId => StatusCode::BAD_REQUEST,

            ApiErrorKind::VapidError(_)
            | ApiErrorKind::Jwt(_)
            | ApiErrorKind::TokenHashValidation(_)
            | ApiErrorKind::InvalidAuthentication
            | ApiErrorKind::InvalidLocalAuth(_) => StatusCode::UNAUTHORIZED,

            ApiErrorKind::InvalidToken | ApiErrorKind::InvalidApiVersion => StatusCode::NOT_FOUND,

            ApiErrorKind::NoUser | ApiErrorKind::NoSubscription => StatusCode::GONE,

            ApiErrorKind::LogCheck => StatusCode::IM_A_TEAPOT,

            ApiErrorKind::General(_)
            | ApiErrorKind::Io(_)
            | ApiErrorKind::Metrics(_)
            | ApiErrorKind::Database(_)
            | ApiErrorKind::EndpointUrl(_)
            | ApiErrorKind::RegistrationSecretHash(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Specify the label to use for metrics reporting.
    pub fn metric_label(&self) -> Option<&'static str> {
        Some(match self {
            ApiErrorKind::PayloadError(_) => "payload_error",
            ApiErrorKind::Router(_) => "router",

            ApiErrorKind::Validation(_) => "validation",
            ApiErrorKind::InvalidEncryption(_) => "invalid_encryption",
            ApiErrorKind::NoTTL => "no_ttl",
            ApiErrorKind::InvalidRouterType => "invalid_router_type",
            ApiErrorKind::InvalidRouterToken => "invalid_router_token",
            ApiErrorKind::InvalidMessageId => "invalid_message_id",

            ApiErrorKind::VapidError(_) => "vapid_error",
            ApiErrorKind::Jwt(_) => "jwt",
            ApiErrorKind::TokenHashValidation(_) => "token_hash_validation",
            ApiErrorKind::InvalidAuthentication => "invalid_authentication",
            ApiErrorKind::InvalidLocalAuth(_) => "invalid_local_auth",

            ApiErrorKind::InvalidToken => "invalid_token",
            ApiErrorKind::InvalidApiVersion => "invalid_api_version",

            ApiErrorKind::NoUser => "no_user",
            ApiErrorKind::NoSubscription => "no_subscription",

            ApiErrorKind::LogCheck => "log_check",

            ApiErrorKind::General(_) => "general",
            ApiErrorKind::Io(_) => "io",
            ApiErrorKind::Metrics(_) => "metrics",
            ApiErrorKind::Database(_) => "database",
            ApiErrorKind::EndpointUrl(_) => "endpoint_url",
            ApiErrorKind::RegistrationSecretHash(_) => "registration_secret_hash",
        })
    }

    /// Don't report all errors to sentry
    pub fn is_sentry_event(&self) -> bool {
        !matches!(
            self,
            // Ignore common webpush errors
            ApiErrorKind::NoTTL | ApiErrorKind::InvalidEncryption(_) |
            // Ignore common VAPID erros
            ApiErrorKind::VapidError(_)
            | ApiErrorKind::Jwt(_)
            | ApiErrorKind::TokenHashValidation(_)
            | ApiErrorKind::InvalidAuthentication
            | ApiErrorKind::InvalidLocalAuth(_) |
            // Ignore missing or invalid user errors
            ApiErrorKind::NoUser | ApiErrorKind::NoSubscription |
            // Ignore overflow errors
            ApiErrorKind::Router(RouterError::TooMuchData(_)),
        )
    }

    /// Get the associated error number
    pub fn errno(&self) -> Option<usize> {
        match self {
            ApiErrorKind::Router(e) => e.errno(),

            ApiErrorKind::Validation(e) => errno_from_validation_errors(e),

            ApiErrorKind::InvalidToken | ApiErrorKind::InvalidApiVersion => Some(102),

            ApiErrorKind::NoUser => Some(103),

            ApiErrorKind::PayloadError(error)
                if matches!(error.as_error(), Some(PayloadError::Overflow))
                    || matches!(error.as_error(), Some(JsonPayloadError::Overflow { .. })) =>
            {
                Some(104)
            }

            ApiErrorKind::NoSubscription => Some(106),

            ApiErrorKind::InvalidRouterType => Some(108),

            ApiErrorKind::VapidError(_)
            | ApiErrorKind::TokenHashValidation(_)
            | ApiErrorKind::Jwt(_)
            | ApiErrorKind::InvalidAuthentication
            | ApiErrorKind::InvalidLocalAuth(_) => Some(109),

            ApiErrorKind::InvalidEncryption(_) => Some(110),

            ApiErrorKind::NoTTL => Some(111),

            ApiErrorKind::LogCheck => Some(999),

            ApiErrorKind::General(_)
            | ApiErrorKind::Io(_)
            | ApiErrorKind::Metrics(_)
            | ApiErrorKind::Database(_)
            | ApiErrorKind::PayloadError(_)
            | ApiErrorKind::InvalidRouterToken
            | ApiErrorKind::RegistrationSecretHash(_)
            | ApiErrorKind::EndpointUrl(_)
            | ApiErrorKind::InvalidMessageId => None,
        }
    }
}

/// temporary bridge between errors.
// TODO: move to ApcError
impl From<ApiErrorKind> for ApcErrorKind {
    fn from(kind: ApiErrorKind) -> ApcErrorKind {
        match kind {
            ApiErrorKind::General(e) => ApcErrorKind::GeneralError(e),
            ApiErrorKind::Io(e) => ApcErrorKind::Io(e),
            ApiErrorKind::Metrics(e) => ApcErrorKind::MetricError(e),
            ApiErrorKind::Validation(e) => ApcErrorKind::EndpointError("Validation", e.to_string()),
            ApiErrorKind::PayloadError(e) => ApcErrorKind::PayloadError(e.to_string()),
            ApiErrorKind::VapidError(e) => ApcErrorKind::EndpointError("Vapid", e.to_string()),
            ApiErrorKind::Router(e) => ApcErrorKind::EndpointError("Router", e.to_string()),
            ApiErrorKind::Jwt(e) => ApcErrorKind::EndpointError("JWT", e.to_string()),
            ApiErrorKind::TokenHashValidation(e) => ApcErrorKind::TokenHashValidation(e),
            ApiErrorKind::RegistrationSecretHash(e) => ApcErrorKind::RegistrationSecretHash(e),
            ApiErrorKind::EndpointUrl(e) => e.kind,
            ApiErrorKind::Database(e) => ApcErrorKind::DatabaseError(e.to_string()),
            ApiErrorKind::InvalidToken => {
                ApcErrorKind::EndpointError("InvalidToken", "".to_string())
            }
            ApiErrorKind::NoUser => ApcErrorKind::EndpointError("NoUser", "".to_string()),
            ApiErrorKind::NoSubscription => {
                ApcErrorKind::EndpointError("NoSubscription", "".to_string())
            }
            ApiErrorKind::InvalidEncryption(e) => {
                ApcErrorKind::EndpointError("InvalidEncryption", e)
            }
            ApiErrorKind::InvalidApiVersion => {
                ApcErrorKind::EndpointError("InvalidApiVersion", "".to_string())
            }
            ApiErrorKind::NoTTL => ApcErrorKind::EndpointError("NoTTL", "".to_string()),
            ApiErrorKind::InvalidRouterType => {
                ApcErrorKind::EndpointError("InvalidRouterType", "".to_string())
            }
            ApiErrorKind::InvalidRouterToken => {
                ApcErrorKind::EndpointError("InvalidRouterToken", "".to_string())
            }
            ApiErrorKind::InvalidMessageId => {
                ApcErrorKind::EndpointError("InvalidMessageId", "".to_string())
            }
            ApiErrorKind::InvalidAuthentication => {
                ApcErrorKind::EndpointError("InvalidAuthentication", "".to_string())
            }
            ApiErrorKind::InvalidLocalAuth(e) => ApcErrorKind::EndpointError("InvalidLocalAuth", e),
            ApiErrorKind::LogCheck => {
                ApcErrorKind::EndpointError("LogCheck", "testing 1,2,3".to_string())
            }
        }
    }
}

// Print out the error and backtrace, including source errors
impl Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Error: {}\nBacktrace: \n{:?}", self.kind, self.backtrace)?;

        // Go down the chain of errors
        let mut error: &dyn Error = &self.kind;
        while let Some(source) = error.source() {
            write!(f, "\n\nCaused by: {source}")?;
            error = source;
        }

        Ok(())
    }
}

impl Error for ApiError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.kind.source()
    }
}

// Forward From impls to ApiError from ApiErrorKind. Because From is reflexive,
// this impl also takes care of From<ApiErrorKind>.
impl<T> From<T> for ApiError
where
    ApiErrorKind: From<T>,
{
    fn from(item: T) -> Self {
        ApiError {
            kind: ApiErrorKind::from(item),
            backtrace: Backtrace::new(),
        }
    }
}

impl ResponseError for ApiError {
    fn status_code(&self) -> StatusCode {
        self.kind.status()
    }

    fn error_response(&self) -> HttpResponse {
        let mut builder = HttpResponse::build(self.kind.status());

        if self.status_code() == 410 {
            builder.insert_header(("Cache-Control", "max-age=86400"));
        }

        builder.json(self)
    }
}

impl Serialize for ApiError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let status = self.kind.status();
        let mut map = serializer.serialize_map(Some(5))?;

        map.serialize_entry("code", &status.as_u16())?;
        map.serialize_entry("errno", &self.kind.errno())?;
        map.serialize_entry("error", &status.canonical_reason())?;
        map.serialize_entry("message", &self.kind.to_string())?;
        map.serialize_entry("more_info", ERROR_URL)?;
        map.end()
    }
}

/// Get the error number from validation errors. If multiple errors are present,
/// the first one with a valid error code is used.
fn errno_from_validation_errors(e: &ValidationErrors) -> Option<usize> {
    // Build an iterator over the error numbers, then get the first one
    e.errors()
        .values()
        .flat_map(|error| match error {
            ValidationErrorsKind::Struct(inner_errors) => {
                Box::new(errno_from_validation_errors(inner_errors).into_iter())
                    as Box<dyn Iterator<Item = usize>>
            }
            ValidationErrorsKind::List(indexed_errors) => Box::new(
                indexed_errors
                    .values()
                    .filter_map(|errors| errno_from_validation_errors(errors)),
            )
                as Box<dyn Iterator<Item = usize>>,
            ValidationErrorsKind::Field(errors) => {
                Box::new(errors.iter().filter_map(|error| error.code.parse().ok()))
                    as Box<dyn Iterator<Item = usize>>
            }
        })
        .next()
}

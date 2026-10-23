//! Who makes a request and what they may do.
//!
//! Every request carries a [`Principal`]. On the desktop it is the implicit
//! local user, who may do everything; accounts and per-project roles of a
//! country server plug in here without touching the handlers.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::state::AppState;

/// Rights on a project, ordered: each includes the previous ones.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Read datasets, jobs, runs and previews.
    Viewer,
    /// Import, tag, save jobs, start and cancel runs.
    Editor,
    /// Rename and delete the project, manage its members.
    Owner,
}

/// What a command or query needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "scope", content = "role", rename_all = "snake_case")]
pub enum Requirement {
    /// Any signed-in user (create a project, list analyses).
    User,
    /// A role on the project named in the request.
    Project(Role),
    /// A server administrator (manage accounts).
    Admin,
}

/// The user behind a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Principal {
    /// User name; `local` on the desktop.
    pub user: String,
    /// Server administrator.
    pub admin: bool,
}

impl Principal {
    /// The implicit user of a desktop server.
    pub fn local() -> Self {
        Self {
            user: "local".into(),
            admin: true,
        }
    }

    /// Check `requirement`, for `project` when it names one.
    pub fn authorize(
        &self,
        requirement: Requirement,
        project: Option<&str>,
    ) -> Result<(), ApiError> {
        // Desktop: the local administrator may do everything. Accounts and
        // project roles (country server) are checked here.
        if self.admin {
            return Ok(());
        }
        match (requirement, project) {
            (Requirement::User, _) => Ok(()),
            _ => Err(ApiError::Forbidden(format!(
                "{} may not do this{}",
                self.user,
                project
                    .map(|p| format!(" in project {p}"))
                    .unwrap_or_default()
            ))),
        }
    }
}

impl FromRequestParts<AppState> for Principal {
    type Rejection = ApiError;

    async fn from_request_parts(_parts: &mut Parts, _state: &AppState) -> Result<Self, ApiError> {
        Ok(Principal::local())
    }
}

// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

pub mod error;
pub mod extract;

pub use error::{AppJson, HttpError};
pub use extract::{AuthenticatedAccount, ClientIp};

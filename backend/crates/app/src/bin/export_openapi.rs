// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Writes the OpenAPI contract to `contracts/openapi.json` (repository root), or, with `--check`,
//! verifies that the committed file matches what the code generates. The path is resolved from
//! the crate manifest, so it does not depend on the directory the command is run from.

use baze_app::openapi::ApiDoc;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use utoipa::OpenApi;

fn contract_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../contracts/openapi.json")
}

fn main() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let check = std::env::args().skip(1).any(|arg| arg == "--check");

    let mut json = ApiDoc::openapi().to_pretty_json()?;
    json.push('\n');

    let target = contract_path();

    if check {
        return Ok(match fs::read_to_string(&target) {
            Ok(current) if current == json => {
                println!("contracts/openapi.json is up to date");
                ExitCode::SUCCESS
            }
            _ => {
                eprintln!(
                    "contracts/openapi.json is out of date. Regenerate it with:\n  cd backend && cargo run -p baze-app --bin export-openapi"
                );
                ExitCode::FAILURE
            }
        });
    }

    if let Some(parent) = target.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(&target, &json)?;
    println!("OpenAPI schema exported to contracts/openapi.json");
    Ok(ExitCode::SUCCESS)
}

// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use baze_app::openapi::ApiDoc;
use std::fs;
use std::path::Path;
use utoipa::OpenApi;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let openapi = ApiDoc::openapi();
    let json = openapi.to_pretty_json()?;

    // Intentar escribir en contracts/openapi.json
    let target_path = Path::new("contracts/openapi.json");
    let fallback_path = Path::new("../../contracts/openapi.json");

    if let Some(parent) = target_path.parent() {
        if parent.exists() {
            fs::write(target_path, &json)?;
            println!("OpenAPI schema exported to {}", target_path.display());
            return Ok(());
        }
    }

    if let Some(parent) = fallback_path.parent() {
        if parent.exists() {
            fs::write(fallback_path, &json)?;
            println!("OpenAPI schema exported to {}", fallback_path.display());
            return Ok(());
        }
    }

    println!("{}", json);
    Ok(())
}

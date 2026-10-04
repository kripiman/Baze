// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use baze_app::openapi::ApiDoc;
use std::fs;
use std::path::Path;
use utoipa::OpenApi;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let openapi = ApiDoc::openapi();
    let json = openapi.to_pretty_json()?;

    let target_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../contracts/openapi.json");
    if let Some(parent) = target_path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent)?;
    }

    fs::write(&target_path, &json)?;
    println!("OpenAPI schema exported to {}", target_path.display());
    Ok(())
}

// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

fn main() {
    // `sqlx::migrate!` embeds the migration files at compile time but only notices changes to files it
    // already knew about. Watching the directory makes a newly added migration trigger a rebuild, so
    // the binary can never ship with a stale set.
    println!("cargo:rerun-if-changed=../../migrations");
}
